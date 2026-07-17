//! Evaluation orchestration: full eval loop.

use crate::actor::Actor;
use crate::dataset::{Category, Question};
use crate::ingest::{self, IngestStrategy};
use crate::inspect;
use crate::judge::Judge;
use crate::report::{EvalReport, RunStatus};
use crate::retrieval::{self, RetrievalConfig, RetrievalPath};
use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::PathBuf;

/// Evaluation configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvalConfig {
    pub dataset_path: PathBuf,
    pub work_dir: PathBuf,
    pub max_questions: Option<usize>,
    pub categories: Option<Vec<Category>>,
    pub seed: u64,
    pub ingest_strategy: IngestStrategy,
    pub retrieval: RetrievalConfig,
    /// Which retrieval path to use (tact or graph).
    pub retrieval_path: RetrievalPath,
    /// Use cascade retrieval (L1→L2→L3) instead of direct recall.
    #[serde(default)]
    pub use_cascade: bool,
    /// If set, write per-memory signal score records to this JSONL path.
    #[serde(default)]
    pub dump_scores_path: Option<PathBuf>,
    /// Save partial results every N questions.
    pub checkpoint_interval: usize,
    /// When Some, overrides per-question shape routing — all questions use this path.
    /// When None and use_cascade is true, shape routing is active.
    #[serde(default)]
    pub retrieval_path_override: Option<RetrievalPath>,
    /// Filter to a single question by ID (for targeted pre-validation).
    #[serde(default)]
    pub question_id: Option<String>,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            dataset_path: PathBuf::from("longmemeval_s.json"),
            work_dir: PathBuf::from("eval-work"),
            max_questions: None,
            categories: None,
            seed: 42,
            ingest_strategy: IngestStrategy::default(),
            retrieval: RetrievalConfig::default(),
            retrieval_path: RetrievalPath::default(),
            use_cascade: false,
            dump_scores_path: None,
            checkpoint_interval: 10,
            retrieval_path_override: None,
            question_id: None,
        }
    }
}

/// Per-call cost estimate for a model.
fn model_cost_per_call(model: &str) -> f64 {
    match model {
        "claude-sonnet-4-6" => 0.04,
        m if m.starts_with("claude-haiku") => 0.008,
        _ => 0.0005, // local models, conservative undercount
    }
}

/// Estimate the cost of running the eval with given models.
pub fn estimate_cost_for_models(
    question_count: usize,
    actor_model: &str,
    judge_model: &str,
) -> f64 {
    let per_question = model_cost_per_call(actor_model) + model_cost_per_call(judge_model);
    question_count as f64 * per_question
}

/// Estimate the cost of running the eval (default Sonnet models).
pub fn estimate_cost(question_count: usize) -> f64 {
    estimate_cost_for_models(question_count, "claude-sonnet-4-6", "claude-sonnet-4-6")
}

/// Truncate an actor answer at the first prompt-continuation artifact.
///
/// The actor sometimes does not stop after answering and instead fabricates a
/// follow-up turn — echoing the prompt scaffold ("Question:", "## Question",
/// "Now answer the following question…") and then answering an unrelated,
/// invented question. Downstream, the judge reads the full `predicted` blob and
/// grades that trailing turn against the real question, scoring a correct answer
/// wrong. Cutting at the first such marker recovers the actual answer. Markers
/// are line-leading prompt scaffolding that a genuine answer is extremely
/// unlikely to contain.
fn strip_actor_continuation(answer: &str) -> String {
    const MARKERS: &[&str] = &[
        "\nQuestion:",
        "\n## Question",
        "\nNow answer the following question",
    ];
    let cut = MARKERS
        .iter()
        .filter_map(|m| answer.find(m))
        .min()
        .unwrap_or(answer.len());
    // Also drop any trailing scaffold separator ("---") and whitespace left
    // immediately before the cut.
    answer[..cut]
        .trim_end_matches(|c: char| c == '-' || c.is_whitespace())
        .to_string()
}

/// The main evaluator.
pub struct AccuracyEval {
    config: EvalConfig,
    actor: Box<dyn Actor>,
    judge: Box<dyn Judge>,
    /// Optional description map for FTS enrichment.
    descriptions: Option<crate::describe::DescriptionMap>,
    /// Optional query expansion config.
    expansion: Option<crate::expansion::ExpansionConfig>,
}

/// Result of evaluating a single question.
struct SingleResult {
    correct: bool,
    predicted: String,
    memory_count: usize,
    memory_keys: Vec<String>,
    reasoning: Option<String>,
    duration_ms: u64,
    /// Raw memory hits for signal-score dumping.
    raw_hits: Vec<spectral_ingest::MemoryHit>,
    /// Cascade telemetry (populated when use_cascade is true).
    cascade_telemetry: Option<retrieval::CascadeTelemetry>,
    /// Strategy routing telemetry.
    strategy_telemetry: Option<crate::report::StrategyTelemetry>,
    /// Total retries across actor + judge.
    retry_count: u32,
    /// Outcome classification.
    outcome_class: crate::report::OutcomeClass,
    /// Rendered memories text for replay.
    actor_context: String,
    /// Per-question efficiency metrics.
    efficiency: Option<crate::report::EfficiencyMetrics>,
}

impl AccuracyEval {
    pub fn new(config: EvalConfig, actor: Box<dyn Actor>, judge: Box<dyn Judge>) -> Self {
        Self {
            config,
            actor,
            judge,
            descriptions: None,
            expansion: None,
        }
    }

    pub fn with_expansion(mut self, config: crate::expansion::ExpansionConfig) -> Self {
        self.expansion = Some(config);
        self
    }

    /// Set the description map for FTS enrichment.
    pub fn with_descriptions(mut self, descriptions: crate::describe::DescriptionMap) -> Self {
        self.descriptions = Some(descriptions);
        self
    }

    /// Run the full evaluation.
    pub fn run(&self) -> Result<EvalReport> {
        let questions_all = crate::dataset::load_dataset(&self.config.dataset_path)?;
        let questions = self.filter_questions(&questions_all);

        eprintln!(
            "Running {} questions (actor: {}, judge: {})",
            questions.len(),
            self.actor.name(),
            self.judge.name()
        );

        let retrieval_path_label = match self.config.retrieval_path {
            RetrievalPath::TopkFts => "topk_fts",
            RetrievalPath::Tact => "tact",
            RetrievalPath::Graph => "graph",
            RetrievalPath::Cascade => "cascade",
        };
        let checkpoint_path = self.config.work_dir.join("checkpoint.json");
        // Config fingerprint: a checkpoint may only be resumed by a run with
        // the SAME configuration. Includes the question filter, routing, and
        // all SPECTRAL_* env levers — two arms of an A/B comparison sharing a
        // work_dir must never inherit each other's results (this silently
        // produced identical A/B arms before the fingerprint existed).
        let mut env_levers: Vec<String> = std::env::vars()
            .filter(|(k, _)| k.starts_with("SPECTRAL_"))
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        env_levers.sort();
        let config_fingerprint = blake3::hash(
            format!(
                "{:?}|{retrieval_path_label}|{}|{:?}|{}",
                self.config.question_id,
                self.config.use_cascade,
                self.config.retrieval_path_override,
                env_levers.join(",")
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        // Resume: if a checkpoint exists for this work-dir, continue from it
        // rather than restarting at question 0. The loaded report carries the
        // prior per-question results and counters; completed questions are
        // skipped below and remain in the final report.
        let mut report = match crate::report::load_report(&checkpoint_path) {
            Ok(existing)
                if !existing.results.is_empty()
                    // Empty fingerprint = legacy checkpoint: allow resume.
                    && !existing.config_fingerprint.is_empty()
                    && existing.config_fingerprint != config_fingerprint =>
            {
                eprintln!(
                    "Checkpoint at {} belongs to a DIFFERENT configuration — starting fresh.",
                    checkpoint_path.display()
                );
                let mut r = EvalReport::new(self.actor.name(), self.judge.name());
                r.retrieval_path = retrieval_path_label.into();
                r.config_fingerprint = config_fingerprint.clone();
                r
            }
            Ok(mut existing) if !existing.results.is_empty() => {
                eprintln!(
                    "Resuming from {}: {} completed question(s) will be skipped.",
                    checkpoint_path.display(),
                    existing.results.len()
                );
                // A prior run may have halted on errors; a clean resume must
                // not inherit that terminal status.
                existing.run_status = RunStatus::Completed;
                existing
            }
            _ => {
                let mut r = EvalReport::new(self.actor.name(), self.judge.name());
                r.retrieval_path = retrieval_path_label.into();
                r.config_fingerprint = config_fingerprint.clone();
                r
            }
        };
        report.config_fingerprint = config_fingerprint;
        let pb = ProgressBar::new(questions.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );

        let completed: HashSet<String> = report
            .results
            .iter()
            .map(|r| r.question_id.clone())
            .collect();
        let mut consecutive_errors: usize = 0;
        const MAX_CONSECUTIVE_ERRORS: usize = 3;

        // Open score dump file if requested
        let mut score_writer: Option<std::io::BufWriter<std::fs::File>> =
            self.config.dump_scores_path.as_ref().map(|p| {
                std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new("."))).ok();
                std::io::BufWriter::new(std::fs::File::create(p).expect("create score dump file"))
            });

        for question in questions.iter() {
            if completed.contains(&question.question_id) {
                pb.inc(1);
                continue;
            }

            let category = match Category::from_question_type(&question.question_type) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("warn: skipping {} — {e}", question.question_id);
                    pb.inc(1);
                    continue;
                }
            };

            match self.eval_single(question, category) {
                Ok(r) => {
                    let is_failure = r.outcome_class != crate::report::OutcomeClass::Ok;
                    if is_failure {
                        consecutive_errors += 1;
                    } else {
                        consecutive_errors = 0;
                    }
                    if let Some(ref mut w) = score_writer {
                        let _ = inspect::write_score_records(
                            w,
                            &question.question_id,
                            &r.raw_hits,
                            r.correct,
                        );
                    }
                    let answer_text = question.answer_text();
                    report.record(
                        &question.question_id,
                        category,
                        r.correct,
                        &question.question,
                        &r.predicted,
                        &answer_text,
                        r.reasoning,
                        r.memory_count,
                        r.memory_keys,
                        r.duration_ms,
                        r.cascade_telemetry,
                        r.strategy_telemetry,
                        r.retry_count,
                        r.outcome_class,
                        Some(r.actor_context),
                        question.question_date.clone(),
                        r.efficiency,
                    );

                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        eprintln!(
                            "[FATAL] {} consecutive errors. Halting run. Partial report saved.",
                            consecutive_errors
                        );
                        report.run_status = RunStatus::HaltedOnErrors { consecutive_errors };
                        break;
                    }
                }
                Err(e) => {
                    // Non-API error (ingest/retrieval failure)
                    consecutive_errors += 1;
                    eprintln!("[ERROR] {}: {e}", question.question_id);
                    let answer_text = question.answer_text();
                    report.record(
                        &question.question_id,
                        category,
                        false,
                        &question.question,
                        &format!("[error: {e}]"),
                        &answer_text,
                        Some(format!("Local error: {e}")),
                        0,
                        Vec::new(),
                        0,
                        None,
                        None,
                        0,
                        crate::report::OutcomeClass::TransportFailure,
                        None,
                        question.question_date.clone(),
                        None,
                    );

                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        eprintln!(
                            "[FATAL] {} consecutive errors. Halting run. Partial report saved.",
                            consecutive_errors
                        );
                        report.run_status = RunStatus::HaltedOnErrors { consecutive_errors };
                        break;
                    }
                }
            }

            pb.inc(1);

            // Durable checkpoint after every completed question, so an
            // interruption resumes where it stopped (losing at most the
            // question in flight). Consumed by the resume path above.
            let mut cp = report.clone();
            cp.finalize();
            let _ = crate::report::save_report(&cp, &checkpoint_path);
        }

        pb.finish_with_message("done");
        report.finalize();
        Ok(report)
    }

    /// Run a single question: ingest, retrieve, act, judge.
    fn eval_single(&self, question: &Question, category: Category) -> Result<SingleResult> {
        let start = std::time::Instant::now();
        let brain_dir = self
            .config
            .work_dir
            .join(format!("brain_{}", question.question_id));

        // Ingest
        let brain = ingest::ingest_question(question, &brain_dir, self.config.ingest_strategy)?;

        // Apply descriptions for FTS enrichment (if provided)
        if let Some(ref descs) = self.descriptions {
            let _ = crate::describe::apply_descriptions(&brain, descs);
        }

        // Query expansion: augment question with synonym/domain terms for FTS
        let (retrieval_query, expansion_usage, expansion_wall_ms) =
            if let Some(ref exp_config) = self.expansion {
                let t = std::time::Instant::now();
                match crate::expansion::expand_query(&question.question, exp_config) {
                    Ok((expanded, usage)) => {
                        let wall = t.elapsed().as_millis() as u64;
                        (expanded, usage, wall)
                    }
                    Err(e) => {
                        eprintln!(
                            "  [expansion] {}: expansion failed, using original: {e}",
                            question.question_id
                        );
                        (
                            question.question.clone(),
                            None,
                            t.elapsed().as_millis() as u64,
                        )
                    }
                }
            } else {
                (question.question.clone(), None, 0)
            };

        // Classify question shape for routing (use original question, not expanded)
        let qtype = retrieval::QuestionType::classify(&question.question);

        // Determine effective retrieval path for this question.
        //
        // Precedence:
        // 1. Explicit --retrieval-path override → all questions use that path.
        // 2. --use-cascade without explicit path → shape routing (Temporal→topk_fts, rest→cascade).
        // 3. Neither → use config.retrieval_path default (topk_fts).
        let effective_path = if let Some(override_path) = self.config.retrieval_path_override {
            override_path
        } else if self.config.use_cascade {
            qtype.retrieval_path()
        } else {
            self.config.retrieval_path
        };

        let strategy_telemetry = if self.config.use_cascade {
            Some(crate::report::StrategyTelemetry {
                shape: format!("{qtype:?}"),
                prompt_template: qtype.prompt_template().to_string(),
                retrieval_path_chosen: format!("{effective_path:?}"),
            })
        } else {
            None
        };

        // Retrieve — get raw hits for score dumping, formatted strings for actor
        // Use expanded query for retrieval, original question for actor prompt
        let question_date = question.question_date.as_deref();
        let retrieval_start = std::time::Instant::now();
        let (mut memories, raw_hits, cascade_telemetry) = match effective_path {
            RetrievalPath::TopkFts => {
                let (formatted, hits) = retrieval::retrieve_topk_fts(
                    &brain,
                    &retrieval_query,
                    &self.config.retrieval,
                    question_date,
                )?;
                (formatted, hits, None)
            }
            RetrievalPath::Tact => {
                let result = brain.recall_local(&retrieval_query)?;
                let hits: Vec<_> = result
                    .memory_hits
                    .into_iter()
                    .take(self.config.retrieval.max_results)
                    .collect();
                let formatted: Vec<String> = hits.iter().map(retrieval::format_hit).collect();
                (formatted, hits, None)
            }
            RetrievalPath::Graph => {
                let formatted =
                    retrieval::retrieve_graph(&brain, &retrieval_query, &self.config.retrieval)?;
                (formatted, Vec::new(), None)
            }
            RetrievalPath::Cascade => {
                let (formatted, hits, telemetry) = retrieval::retrieve_cascade(
                    &brain,
                    &retrieval_query,
                    &self.config.retrieval,
                    question_date,
                )?;
                (formatted, hits, Some(telemetry))
            }
        };
        let retrieval_wall_ms = retrieval_start.elapsed().as_millis() as u64;
        let memory_count = memories.len();
        // Extract keys from raw_hits when available (most reliable).
        // Fallback: extract session IDs from "--- Session <id> ---" headers
        // or keys from "[date] [wing/hall] key: content" flat format.
        let memory_keys: Vec<String> = if !raw_hits.is_empty() {
            raw_hits.iter().map(|h| h.key.clone()).collect()
        } else {
            memories
                .iter()
                .filter_map(|m| {
                    // Session-grouped format: "--- Session <id> (<date>) ---"
                    if m.starts_with("--- Session ") {
                        let rest = m.strip_prefix("--- Session ")?;
                        let id = rest.split(' ').next()?;
                        return Some(id.to_string());
                    }
                    // Flat format: "[date] [wing/hall] key: content"
                    let first_close = m.find("] ")?;
                    let after_first = &m[first_close + 2..];
                    let second_close = after_first.find("] ")?;
                    let key_and_content = &after_first[second_close + 2..];
                    key_and_content.split(": ").next().map(|k| k.to_string())
                })
                .collect()
        };

        // Optional read-time consolidation pre-pass (SPECTRAL_CONSOLIDATE_CONTEXT=1):
        // one sparse haiku call dedups cross-session mentions into an entity-keyed
        // atom list, prepended to the context (the raw sessions remain below for
        // grounding). Falls back silently to the flat context on any error.
        if std::env::var("SPECTRAL_CONSOLIDATE_CONTEXT")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            if let Some(atoms) =
                crate::consolidate::consolidate_context(&question.question, &memories)
            {
                memories.insert(
                    0,
                    format!(
                        "=== CONSOLIDATED CANDIDATE ITEMS (deduplicated across sessions; use as the \
candidate set, then verify against the raw sessions below) ===\n{atoms}"
                    ),
                );
            }
        }

        // Act — with retry on transient failures
        let actor_context = memories.join("\n");
        let question_date_str = question.question_date.as_deref().unwrap_or("unknown");
        let actor_start = std::time::Instant::now();
        let actor_outcome = crate::retry::with_retry(4, &question.question_id, "actor", || {
            self.actor
                .answer(&question.question, question_date_str, &memories, qtype)
        });
        let actor_wall_ms = actor_start.elapsed().as_millis() as u64;

        let (predicted, actor_usage, mut total_retries, outcome_class) = match actor_outcome {
            crate::retry::CallOutcome::Success { value, retry_count } => {
                let (text, usage) = value;
                // The actor occasionally runs past its answer and fabricates a
                // follow-up "Question:/Answer:" turn (prompt-continuation leak).
                // The judge would then grade that trailing turn instead of the
                // real answer. Truncate it here so both the judged text and the
                // stored `predicted` contain only the actual answer.
                (
                    strip_actor_continuation(&text),
                    usage,
                    retry_count,
                    crate::report::OutcomeClass::Ok,
                )
            }
            crate::retry::CallOutcome::TransportFailure { error, retry_count } => {
                eprintln!("[TRANSPORT] {}: {error}", question.question_id);
                let _ = std::fs::remove_dir_all(&brain_dir);
                return Ok(SingleResult {
                    correct: false,
                    predicted: format!("[error: {error}]"),
                    memory_count,
                    memory_keys,
                    reasoning: Some(format!("Actor transport failure: {error}")),
                    duration_ms: start.elapsed().as_millis() as u64,
                    raw_hits,
                    cascade_telemetry,
                    strategy_telemetry,
                    retry_count,
                    outcome_class: crate::report::OutcomeClass::TransportFailure,
                    actor_context: actor_context.clone(),
                    efficiency: None,
                });
            }
            crate::retry::CallOutcome::AuthFailure { error } => {
                eprintln!("[AUTH] {}: {error}", question.question_id);
                let _ = std::fs::remove_dir_all(&brain_dir);
                return Ok(SingleResult {
                    correct: false,
                    predicted: format!("[error: {error}]"),
                    memory_count,
                    memory_keys,
                    reasoning: Some(format!("Auth failure: {error}")),
                    duration_ms: start.elapsed().as_millis() as u64,
                    raw_hits,
                    cascade_telemetry,
                    strategy_telemetry,
                    retry_count: 0,
                    outcome_class: crate::report::OutcomeClass::AuthFailure,
                    actor_context: actor_context.clone(),
                    efficiency: None,
                });
            }
        };

        // Judge — with retry on transient failures
        let answer_text = question.answer_text();
        let judge_outcome = crate::retry::with_retry(4, &question.question_id, "judge", || {
            self.judge
                .grade(&question.question, &predicted, &answer_text, category)
        });

        let (grade, judge_usage, outcome_class) = match judge_outcome {
            crate::retry::CallOutcome::Success { value, retry_count } => {
                total_retries += retry_count;
                let (g, u) = value;
                (g, u, outcome_class)
            }
            crate::retry::CallOutcome::TransportFailure { error, retry_count } => {
                eprintln!("[TRANSPORT] {} judge: {error}", question.question_id);
                total_retries += retry_count;
                let _ = std::fs::remove_dir_all(&brain_dir);
                return Ok(SingleResult {
                    correct: false,
                    predicted,
                    memory_count,
                    memory_keys,
                    reasoning: Some(format!("Judge transport failure: {error}")),
                    duration_ms: start.elapsed().as_millis() as u64,
                    raw_hits,
                    cascade_telemetry,
                    strategy_telemetry,
                    retry_count: total_retries,
                    outcome_class: crate::report::OutcomeClass::TransportFailure,
                    actor_context: actor_context.clone(),
                    efficiency: None,
                });
            }
            crate::retry::CallOutcome::AuthFailure { error } => {
                eprintln!("[AUTH] {} judge: {error}", question.question_id);
                let _ = std::fs::remove_dir_all(&brain_dir);
                return Ok(SingleResult {
                    correct: false,
                    predicted,
                    memory_count,
                    memory_keys,
                    reasoning: Some(format!("Judge auth failure: {error}")),
                    duration_ms: start.elapsed().as_millis() as u64,
                    raw_hits,
                    cascade_telemetry,
                    strategy_telemetry,
                    retry_count: 0,
                    outcome_class: crate::report::OutcomeClass::AuthFailure,
                    actor_context,
                    efficiency: None,
                });
            }
        };

        // ── Build efficiency metrics ──
        let exp_in = expansion_usage.as_ref().and_then(|u| u.input_tokens);
        let exp_out = expansion_usage.as_ref().and_then(|u| u.output_tokens);
        let act_in = actor_usage.as_ref().and_then(|u| u.input_tokens);
        let act_out = actor_usage.as_ref().and_then(|u| u.output_tokens);
        let jdg_in = judge_usage.as_ref().and_then(|u| u.input_tokens);
        let jdg_out = judge_usage.as_ref().and_then(|u| u.output_tokens);

        // system_tokens = expansion + actor (judge excluded)
        let system_tokens = match (act_in, act_out) {
            (Some(ai), Some(ao)) => Some(exp_in.unwrap_or(0) + exp_out.unwrap_or(0) + ai + ao),
            _ => None,
        };

        // Cost estimation
        let expansion_model = self
            .expansion
            .as_ref()
            .map(|c| c.model.as_str())
            .unwrap_or("");
        let exp_cost = if expansion_usage.is_some() {
            crate::pricing::estimate_call_cost(expansion_model, exp_in, exp_out)
        } else {
            Some(0.0) // no expansion call = zero cost, not unknown
        };
        let actor_cost = crate::pricing::estimate_call_cost(self.actor.name(), act_in, act_out);
        let judge_cost = crate::pricing::estimate_call_cost(self.judge.name(), jdg_in, jdg_out);

        let estimated_cost_usd = match (exp_cost, actor_cost) {
            (Some(e), Some(a)) => Some(e + a),
            _ => None,
        };

        let efficiency = Some(crate::report::EfficiencyMetrics {
            expansion_input_tokens: if self.expansion.is_some() {
                Some(exp_in.unwrap_or(0))
            } else {
                Some(0)
            },
            expansion_output_tokens: if self.expansion.is_some() {
                Some(exp_out.unwrap_or(0))
            } else {
                Some(0)
            },
            actor_input_tokens: act_in,
            actor_output_tokens: act_out,
            judge_input_tokens: jdg_in,
            judge_output_tokens: jdg_out,
            system_tokens_per_query: system_tokens,
            retrieval_wall_ms: Some(retrieval_wall_ms),
            expansion_wall_ms: if self.expansion.is_some() {
                Some(expansion_wall_ms)
            } else {
                Some(0)
            },
            actor_wall_ms: Some(actor_wall_ms),
            estimated_cost_usd,
            judge_cost_usd: judge_cost,
        });

        // Clean up brain directory
        let _ = std::fs::remove_dir_all(&brain_dir);

        Ok(SingleResult {
            correct: grade.correct,
            predicted,
            memory_count,
            memory_keys,
            reasoning: grade.reasoning,
            duration_ms: start.elapsed().as_millis() as u64,
            raw_hits,
            cascade_telemetry,
            strategy_telemetry,
            retry_count: total_retries,
            outcome_class,
            actor_context,
            efficiency,
        })
    }

    fn filter_questions<'a>(&self, questions_all: &'a [Question]) -> Vec<&'a Question> {
        let mut questions: Vec<&Question> = questions_all.iter().collect();

        if let Some(ref qid) = self.config.question_id {
            // Accepts: a single ID, a comma-separated list, or "@path" to a
            // file with one ID per line (targeted Tier-1 replays).
            let ids: HashSet<String> = if let Some(path) = qid.strip_prefix('@') {
                std::fs::read_to_string(path)
                    .map(|s| {
                        s.lines()
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty() && !l.starts_with('#'))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                qid.split(',').map(|s| s.trim().to_string()).collect()
            };
            questions.retain(|q| ids.contains(&q.question_id));
        }

        if let Some(ref cats) = self.config.categories {
            let cat_strs: HashSet<String> = cats.iter().map(|c| c.as_str().to_string()).collect();
            questions.retain(|q| {
                Category::from_question_type(&q.question_type)
                    .map(|cat| cat_strs.contains(cat.as_str()))
                    .unwrap_or(false)
            });
        }

        if let Some(max) = self.config.max_questions {
            questions.truncate(max);
        }

        questions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Actor, MockActor};
    use crate::dataset::{Question, Turn};
    use crate::judge::MockJudge;

    /// Actor that always returns an error.
    struct FailingActor;
    impl Actor for FailingActor {
        fn answer(
            &self,
            _q: &str,
            _d: &str,
            _m: &[String],
            _shape: crate::retrieval::QuestionType,
        ) -> anyhow::Result<(String, Option<crate::report::TokenUsage>)> {
            Err(anyhow::anyhow!("API returned 401: unauthorized"))
        }
        fn name(&self) -> &str {
            "failing"
        }
    }

    /// Actor that fails on the Nth call (0-indexed), succeeds otherwise.
    struct FailNthActor {
        fail_on: usize,
        call_count: std::sync::Mutex<usize>,
    }
    impl FailNthActor {
        fn new(fail_on: usize) -> Self {
            Self {
                fail_on,
                call_count: std::sync::Mutex::new(0),
            }
        }
    }
    impl Actor for FailNthActor {
        fn answer(
            &self,
            _q: &str,
            _d: &str,
            _m: &[String],
            _shape: crate::retrieval::QuestionType,
        ) -> anyhow::Result<(String, Option<crate::report::TokenUsage>)> {
            let mut count = self.call_count.lock().unwrap();
            let current = *count;
            *count += 1;
            if current == self.fail_on {
                Err(anyhow::anyhow!("API returned 429: rate limited"))
            } else {
                Ok(("test answer".into(), None))
            }
        }
        fn name(&self) -> &str {
            "fail-nth"
        }
    }

    /// Actor that always fails with a given error message.
    struct AlwaysFailActor {
        error_msg: String,
    }
    impl AlwaysFailActor {
        fn new(msg: &str) -> Self {
            Self {
                error_msg: msg.into(),
            }
        }
    }
    impl Actor for AlwaysFailActor {
        fn answer(
            &self,
            _q: &str,
            _d: &str,
            _m: &[String],
            _shape: crate::retrieval::QuestionType,
        ) -> anyhow::Result<(String, Option<crate::report::TokenUsage>)> {
            Err(anyhow::anyhow!("{}", self.error_msg))
        }
        fn name(&self) -> &str {
            "always-fail"
        }
    }

    fn test_questions() -> Vec<Question> {
        vec![
            Question {
                question_id: "q1".into(),
                question_type: "multi-session".into(),
                question: "What is unknown?".into(),
                answer: serde_json::Value::String("I don't know".into()),
                question_date: Some("2023/05/30 (Tue) 23:40".into()),
                haystack_sessions: vec![vec![
                    Turn {
                        role: "user".into(),
                        content: "Hello there.".into(),
                    },
                    Turn {
                        role: "assistant".into(),
                        content: "Hi!".into(),
                    },
                ]],
                haystack_session_ids: vec!["s1".into()],
                haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
            },
            Question {
                question_id: "q2".into(),
                question_type: "temporal-reasoning".into(),
                question: "What color is the car?".into(),
                answer: serde_json::Value::String("Red".into()),
                question_date: Some("2023/06/01 (Thu) 10:00".into()),
                haystack_sessions: vec![vec![
                    Turn {
                        role: "user".into(),
                        content: "My car is red.".into(),
                    },
                    Turn {
                        role: "assistant".into(),
                        content: "Nice car!".into(),
                    },
                ]],
                haystack_session_ids: vec!["s2".into()],
                haystack_dates: vec!["2023/03/01 (Wed) 12:00".into()],
            },
        ]
    }

    #[test]
    fn full_eval_with_mocks() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        std::fs::write(&ds_path, serde_json::to_string(&test_questions()).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            max_questions: Some(2),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(MockActor::new("test answer")),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        assert_eq!(report.total_questions, 2);
        assert_eq!(report.correct, 2);
        assert!((report.overall_accuracy - 1.0).abs() < 0.001);
    }

    #[test]
    fn strip_actor_continuation_cuts_trailing_fabricated_turns() {
        // 55241a1f shape: real answer, then a fabricated "Question:/Answer:" turn.
        let a = "The total number of comments is 12 + 21 = 33 comments.\n\nQuestion: When did I purchase my Sony FE 24-70mm f/2.8 GM lens?\n\nAnswer:";
        assert_eq!(
            strip_actor_continuation(a),
            "The total number of comments is 12 + 21 = 33 comments."
        );
        // b6025781 shape: prompt-scaffold echo.
        let b = "Here are some meal prep recipes.\n\n---\n\nNow answer the following question based on the conversation context above:\n\n## Question\nWhat gift for mom?";
        assert_eq!(
            strip_actor_continuation(b),
            "Here are some meal prep recipes."
        );
        // 8b9d4367 shape: "## Question" markdown scaffold.
        let c = "The company is Jaipur Rugs.\n## Question\nWhat about Bajaj Auto?";
        assert_eq!(strip_actor_continuation(c), "The company is Jaipur Rugs.");
        // Clean answer is unchanged.
        let clean = "The car is red.";
        assert_eq!(strip_actor_continuation(clean), "The car is red.");
    }

    /// Judge that records exactly what (question, predicted, ground_truth) it was
    /// handed, so we can assert the pairing through the judge call.
    struct RecordingJudge {
        seen: std::sync::Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
    }
    impl crate::judge::Judge for RecordingJudge {
        fn grade(
            &self,
            question: &str,
            predicted: &str,
            ground_truth: &str,
            _category: Category,
        ) -> anyhow::Result<(crate::judge::GradeResult, Option<crate::report::TokenUsage>)>
        {
            self.seen.lock().unwrap().push((
                question.to_string(),
                predicted.to_string(),
                ground_truth.to_string(),
            ));
            Ok((
                crate::judge::GradeResult {
                    correct: true,
                    reasoning: None,
                },
                None,
            ))
        }
        fn name(&self) -> &str {
            "recording"
        }
    }

    #[test]
    fn judge_receives_correct_question_gt_and_sanitized_answer() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let questions = vec![Question {
            question_id: "q-pair".into(),
            question_type: "multi-session".into(),
            question: "What color is the car?".into(),
            answer: serde_json::Value::String("Red".into()),
            question_date: Some("2023/06/01 (Thu) 10:00".into()),
            haystack_sessions: vec![vec![Turn {
                role: "user".into(),
                content: "My car is red and I drive it daily.".into(),
            }]],
            haystack_session_ids: vec!["s1".into()],
            haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
        }];
        std::fs::write(&ds_path, serde_json::to_string(&questions).unwrap()).unwrap();

        // Actor answers correctly, then runs past its answer into a fabricated turn.
        let actor_text =
            "The car is red.\n\nQuestion: When did I buy the Sony lens?\n\nAnswer:".to_string();
        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            max_questions: Some(1),
            checkpoint_interval: 100,
            ..Default::default()
        };
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let judge = Box::new(RecordingJudge { seen: seen.clone() });
        let eval = AccuracyEval::new(config, Box::new(MockActor::new(&actor_text)), judge);
        let report = eval.run().unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "judge called exactly once");
        let (q, predicted, gt) = &seen[0];
        // Pairing: the judge got THIS question and THIS GT (no mispairing).
        assert_eq!(q, "What color is the car?");
        assert_eq!(gt, "Red");
        // Fix: the judge graded the sanitized answer, not the fabricated trailing turn.
        assert_eq!(predicted, "The car is red.");
        // And the stored predicted is sanitized too.
        assert_eq!(report.results[0].predicted, "The car is red.");
    }

    /// Pins the resume contract that the old `ids.clear()` broke: a run whose
    /// work-dir already holds a checkpoint must SKIP the completed questions
    /// (not re-run them) and carry their prior results into the final report.
    #[test]
    fn resume_skips_completed_and_keeps_their_results() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();

        let mk = |id: &str| Question {
            question_id: id.into(),
            question_type: "multi-session".into(),
            question: "What color is the car?".into(),
            answer: serde_json::Value::String("Red".into()),
            question_date: Some("2023/06/01 (Thu) 10:00".into()),
            haystack_sessions: vec![vec![
                Turn {
                    role: "user".into(),
                    content: "My car is red and I drive it every day".into(),
                },
                Turn {
                    role: "assistant".into(),
                    content: "Red cars are very visible on the road.".into(),
                },
            ]],
            haystack_session_ids: vec!["s1".into()],
            haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
        };
        let questions = vec![mk("q-done"), mk("q-todo")];
        std::fs::write(&ds_path, serde_json::to_string(&questions).unwrap()).unwrap();

        // Pre-populate a checkpoint marking q-done complete, with a sentinel
        // `predicted` the MockActor would never produce. If resume re-ran it,
        // the sentinel would be overwritten — which is exactly the regression.
        let cat = Category::from_question_type("multi-session").unwrap();
        let mut pre = crate::report::EvalReport::new("mock", "mock");
        pre.record(
            "q-done",
            cat,
            true,
            "What color is the car?",
            "SENTINEL_PRECOMPUTED",
            "Red",
            Some("precomputed".into()),
            0,
            Vec::new(),
            0,
            None,
            None,
            0,
            crate::report::OutcomeClass::Ok,
            None,
            None,
            None,
        );
        pre.finalize();
        crate::report::save_report(&pre, &work.join("checkpoint.json")).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: work,
            checkpoint_interval: 100,
            ..Default::default()
        };
        let eval = AccuracyEval::new(
            config,
            Box::new(MockActor::new("freshly-run answer")),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();

        // Both questions present exactly once.
        assert_eq!(
            report.results.len(),
            2,
            "final report must contain both questions"
        );
        assert_eq!(report.total_questions, 2);
        let done = report
            .results
            .iter()
            .find(|r| r.question_id == "q-done")
            .expect("completed question must survive in the final report");
        let todo = report
            .results
            .iter()
            .find(|r| r.question_id == "q-todo")
            .expect("missing question must be evaluated");
        // q-done was loaded from the checkpoint, NOT re-evaluated.
        assert_eq!(
            done.predicted, "SENTINEL_PRECOMPUTED",
            "completed question was re-run instead of resumed (the ids.clear() regression)"
        );
        // q-todo was the only question actually run.
        assert_eq!(todo.predicted, "freshly-run answer");
    }

    #[test]
    fn eval_populates_memory_keys_in_report() {
        // Use a question where FTS will match: "car" query against "My car is red" content
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let questions = vec![Question {
            question_id: "q-keys".into(),
            question_type: "multi-session".into(),
            question: "What color is the car?".into(),
            answer: serde_json::Value::String("Red".into()),
            question_date: Some("2023/06/01 (Thu) 10:00".into()),
            haystack_sessions: vec![vec![
                Turn {
                    role: "user".into(),
                    content: "My car is red and I love driving it every day".into(),
                },
                Turn {
                    role: "assistant".into(),
                    content: "That sounds great! Red cars are very visible on the road.".into(),
                },
            ]],
            haystack_session_ids: vec!["s1".into()],
            haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
        }];
        std::fs::write(&ds_path, serde_json::to_string(&questions).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            max_questions: Some(1),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(MockActor::new("Red")),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        assert_eq!(report.results.len(), 1);

        let result = &report.results[0];
        assert!(
            !result.retrieved_memory_keys.is_empty(),
            "retrieved_memory_keys should be populated, got empty"
        );
        // Keys should look like session:turn:N:role format
        for key in &result.retrieved_memory_keys {
            assert!(
                key.contains(':'),
                "memory key should contain ':' separator, got: {key}"
            );
        }
        // memory count should match keys count
        assert_eq!(
            result.retrieved_memory_count,
            result.retrieved_memory_keys.len(),
            "memory_count should equal memory_keys length"
        );
    }

    #[test]
    fn eval_records_failures() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        std::fs::write(&ds_path, serde_json::to_string(&test_questions()).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            max_questions: Some(2),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(MockActor::new("wrong")),
            Box::new(MockJudge::always_fail()),
        );
        let report = eval.run().unwrap();
        assert_eq!(report.correct, 0);
        assert_eq!(report.failures().len(), 2);
    }

    #[test]
    fn cost_estimate_reasonable() {
        let cost = estimate_cost(500);
        assert!(
            cost > 10.0 && cost < 100.0,
            "500 questions should cost $10-100, got ${cost}"
        );
    }

    #[test]
    fn unknown_question_type_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let qs = vec![Question {
            question_id: "q-unknown".into(),
            question_type: "bogus-category".into(),
            question: "Q?".into(),
            answer: serde_json::Value::String("A".into()),
            question_date: None,
            haystack_sessions: vec![vec![Turn {
                role: "user".into(),
                content: "Hello.".into(),
            }]],
            haystack_session_ids: vec!["s1".into()],
            haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
        }];
        std::fs::write(&ds_path, serde_json::to_string(&qs).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(MockActor::new("answer")),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        assert_eq!(report.total_questions, 0, "unknown type should be skipped");
    }

    fn make_n_questions(n: usize) -> Vec<Question> {
        (0..n)
            .map(|i| Question {
                question_id: format!("q{i}"),
                question_type: "multi-session".into(),
                question: format!("Question {i} about topic {i}?"),
                answer: serde_json::Value::String(format!("Answer {i}")),
                question_date: Some("2023/05/30 (Tue) 23:40".into()),
                haystack_sessions: vec![vec![Turn {
                    role: "user".into(),
                    content: format!("Content for question {i} about topic {i}."),
                }]],
                haystack_session_ids: vec![format!("s{i}")],
                haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
            })
            .collect()
    }

    #[test]
    fn eval_halts_on_consecutive_errors() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let qs = make_n_questions(5);
        std::fs::write(&ds_path, serde_json::to_string(&qs).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(FailingActor),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        // Should halt after 3 consecutive errors, not process all 5
        assert_eq!(report.total_questions, 3);
        assert_eq!(
            report.run_status,
            RunStatus::HaltedOnErrors {
                consecutive_errors: 3
            }
        );
    }

    #[test]
    fn eval_recovers_transient_error_via_retry() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let qs = make_n_questions(4);
        std::fs::write(&ds_path, serde_json::to_string(&qs).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            checkpoint_interval: 100,
            ..Default::default()
        };

        // Fail on question index 1 only — retry recovers it
        let eval = AccuracyEval::new(
            config,
            Box::new(FailNthActor::new(1)),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        // All 4 questions should be attempted and recovered
        assert_eq!(report.total_questions, 4);
        assert_eq!(report.run_status, RunStatus::Completed);
        // All correct (the 429 was retried and recovered)
        assert_eq!(report.correct, 4);
        assert_eq!(report.recovered_after_retry, 1);
        // The recovered question should have retry_count > 0
        let retried: Vec<_> = report
            .results
            .iter()
            .filter(|r| r.retry_count > 0)
            .collect();
        assert_eq!(retried.len(), 1, "exactly one question should have retried");
        assert_eq!(retried[0].outcome_class, crate::report::OutcomeClass::Ok);
    }

    #[test]
    fn auth_failure_fails_fast_no_retry() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let qs = make_n_questions(2);
        std::fs::write(&ds_path, serde_json::to_string(&qs).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(AlwaysFailActor::new(
                "API returned 401 Unauthorized: invalid x-api-key",
            )),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        // Auth failures should be tagged, not counted as wrong answers
        assert_eq!(report.auth_failures, 2);
        assert_eq!(report.correct, 0);
        // Accuracy denominator excludes auth failures
        let evaluated = report.total_questions - report.transport_failures - report.auth_failures;
        assert_eq!(evaluated, 0);
        // Outcome class on each result
        for r in &report.results {
            assert_eq!(r.outcome_class, crate::report::OutcomeClass::AuthFailure);
            assert_eq!(r.retry_count, 0, "401 should NOT have retried");
        }
    }

    #[test]
    fn transport_failure_excluded_from_accuracy() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let qs = make_n_questions(2);
        std::fs::write(&ds_path, serde_json::to_string(&qs).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(AlwaysFailActor::new("error sending request for url")),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        assert_eq!(report.transport_failures, 2);
        assert_eq!(report.correct, 0);
        // Transport failures excluded from denominator
        let evaluated = report.total_questions - report.transport_failures - report.auth_failures;
        assert_eq!(evaluated, 0);
        for r in &report.results {
            assert_eq!(
                r.outcome_class,
                crate::report::OutcomeClass::TransportFailure
            );
            assert!(r.retry_count > 0, "transport error should have retried");
        }
    }

    #[test]
    fn cost_estimation_respects_model() {
        let sonnet = estimate_cost_for_models(100, "claude-sonnet-4-6", "claude-sonnet-4-6");
        assert!(
            (sonnet - 8.0).abs() < 0.01,
            "100 Sonnet questions = $8, got {sonnet}"
        );

        let haiku = estimate_cost_for_models(
            100,
            "claude-haiku-4-5-20251001",
            "claude-haiku-4-5-20251001",
        );
        assert!(
            haiku < 2.0,
            "100 Haiku questions should be < $2, got {haiku}"
        );

        let local = estimate_cost_for_models(100, "local-gemma", "local-gemma");
        assert!(
            (local - 0.1).abs() < 0.01,
            "100 local questions = $0.10, got {local}"
        );

        let mixed = estimate_cost_for_models(100, "claude-sonnet-4-6", "claude-haiku-4-5-20251001");
        assert!(
            mixed > haiku && mixed < sonnet,
            "mixed should be between haiku and sonnet"
        );
    }
}
