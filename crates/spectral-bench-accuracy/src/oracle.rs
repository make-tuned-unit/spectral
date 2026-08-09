//! Tier-0 retrieval oracle: retrieval-only evaluation with zero LLM calls.
//!
//! Measures what the memory layer alone delivers — evidence-turn recall,
//! rank of the first evidence turn, retrieved-context size, and a context
//! hash for paired diffing between configurations — without spending a token
//! on actor or judge. This is the gate every retrieval-side change must clear
//! before any paid bench run.
//!
//! # Two different metrics, and only one of them is evidence recall (R15)
//!
//! * **Evidence turns** (`evidence_turns_*`) are the turns LongMemEval itself
//!   labels `has_answer: true`, documented as "used for turn-level memory
//!   recall accuracy evaluation". This is the real quantity: on
//!   LongMemEval-S there are 896 of them across 500 questions (mean 1.79 per
//!   question), and 21 questions carry no label at all, for which the metric
//!   is **undefined, not zero**.
//! * **Answer-session turns** (`answer_session_turns_*`, called
//!   "answer keys"/"key-recall" before 2026-08-07) count *every* turn of a
//!   haystack session whose id starts with `answer_`. That denominator is
//!   10,960 turns — **12.2× larger** than the evidence set. It is
//!   evidence-SESSION turn coverage, a diluted proxy, and it must never be
//!   cited as evidence about retrieval quality.
//!
//! Both are kept: the second only so archived runs remain comparable with the
//! numbers they were published with. See
//! `docs/internal/turn-level-evidence-recall-2026-08-07.md`.

use crate::dataset::{Category, Question};
use crate::ingest::{self, IngestStrategy};
use crate::retrieval::QuestionPrompts;
use crate::retrieval::{self, RetrievalConfig, RetrievalPath};
use anyhow::{Context, Result};
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use spectral_tact::TactConfig;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Configuration for an oracle run.
#[derive(Debug, Clone)]
pub struct OracleConfig {
    pub dataset_path: PathBuf,
    pub work_dir: PathBuf,
    pub output: PathBuf,
    pub max_questions: Option<usize>,
    pub categories: Option<Vec<Category>>,
    pub question_id: Option<String>,
    pub ingest_strategy: IngestStrategy,
    pub retrieval: RetrievalConfig,
    /// Explicit retrieval path. None = per-question shape routing, matching
    /// the published `run --use-cascade` configuration.
    pub retrieval_path_override: Option<RetrievalPath>,
    /// Reuse an existing brain dir instead of re-ingesting. Safe for
    /// ranking-only changes; pass false after any ingest-affecting change.
    pub reuse_brains: bool,
    /// Keep brain dirs after the run for future reuse.
    pub keep_brains: bool,
    /// Config label recorded on every row (e.g. "baseline", "stemming").
    pub label: String,
    /// Optional JSON map {question_id: expanded_query} to replay frozen
    /// query-expansion output without an LLM call.
    pub expansion_cache: Option<PathBuf>,
}

/// Per-question oracle result. One JSONL row per question.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OracleRow {
    pub label: String,
    pub question_id: String,
    pub category: String,
    pub shape: String,
    pub retrieval_path: String,
    pub n_retrieved: usize,
    /// Total turns living in answer sessions. **NOT evidence** — 12.2× the
    /// evidence count on LongMemEval-S. Named `answer_keys_total` before
    /// 2026-08-07 (R15); the alias keeps archived rows loadable.
    #[serde(alias = "answer_keys_total")]
    pub answer_session_turns_total: usize,
    /// Answer-session turns present in the retrieved set. Not evidence recall.
    #[serde(alias = "answer_keys_retrieved")]
    pub answer_session_turns_retrieved: usize,
    /// Distinct answer sessions in the haystack.
    pub answer_sessions_total: usize,
    /// Answer sessions with at least one retrieved turn.
    pub answer_sessions_hit: usize,
    /// 1-based rank of the first answer-session turn in retrieval order.
    /// None = miss.
    #[serde(alias = "rank_first_answer_key")]
    pub rank_first_answer_session_turn: Option<usize>,

    // ── R15 evidence metrics: what LongMemEval actually labels ──
    /// Turns the dataset labels `has_answer: true`. `None` = the metric is
    /// **undefined** for this row (unlabelled question, non-PerTurn ingest,
    /// or a retrieved key set that is not turn-shaped) — never read it as 0.
    #[serde(default)]
    pub evidence_turns_total: Option<usize>,
    /// Labelled evidence turns present in the retrieved set.
    #[serde(default)]
    pub evidence_turns_retrieved: Option<usize>,
    /// 1-based rank of the first evidence turn in retrieval order.
    #[serde(default)]
    pub rank_first_evidence_turn: Option<usize>,
    /// Evidence keys that were not retrieved, sorted. Mean 1.79 evidence
    /// turns per question, so this stays small.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_keys_missed: Vec<String>,
    pub context_chars: usize,
    /// chars/4 heuristic, matching the cost-benchmark accounting.
    pub context_tokens_est: usize,
    /// blake3 of the exact actor context string. Equal hashes between two
    /// configs mean the actor outcome distribution is identical — free pass.
    pub context_hash: String,
    pub retrieval_wall_ms: u64,
    pub retrieved_keys: Vec<String>,
}

/// Open an existing bench brain without re-ingesting. Mirrors the
/// `BrainConfig` used by `ingest::ingest_question` exactly.
fn open_existing_brain(brain_dir: &Path) -> Result<Brain> {
    let ontology_path = brain_dir.join("ontology.toml");
    let mut brain_config = BrainConfig {
        data_dir: brain_dir.to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        // Ablation: `SPECTRAL_NO_FINGERPRINTS=1` skips constellation
        // fingerprint generation at ingest. Requires --fresh-brains.
        fingerprints: Some(std::env::var("SPECTRAL_NO_FINGERPRINTS").is_err()),
        enable_spectrogram: std::env::var("SPECTRAL_BENCH_SPECTROGRAM").is_ok(),
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: Some(TactConfig {
            max_results: 20,
            ..TactConfig::default()
        }),
        ..Default::default()
    };
    crate::env_levers::apply_env_levers(&mut brain_config);
    Ok(Brain::open(brain_config)?)
}

/// The keys of the turns the dataset labels `has_answer: true`, in
/// ingest-key form (built through [`ingest::memory_key`] so the two formats
/// cannot drift).
///
/// Returns `None` — metric undefined, **not zero** — when:
/// * the question carries no `has_answer: true` turn at all (the 21
///   LongMemEval `_abs` abstention questions, and every LoCoMo-converted
///   dataset, which ships no labels); or
/// * the ingest strategy is not `PerTurn`. Under `PerSession` every turn of
///   a session collapses to one `{sid}:session` key, so the quantity would
///   be evidence *sessions* while the field name says turns. We refuse
///   rather than publish a field whose name is false.
pub fn evidence_keys(question: &Question, strategy: IngestStrategy) -> Option<BTreeSet<String>> {
    if strategy != IngestStrategy::PerTurn {
        return None;
    }
    let mut set = BTreeSet::new();
    for (s_idx, session) in question.haystack_sessions.iter().enumerate() {
        let sid = question
            .haystack_session_ids
            .get(s_idx)
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        for (t_idx, turn) in session.iter().enumerate() {
            if turn.has_answer == Some(true) {
                set.insert(ingest::memory_key(strategy, sid, t_idx, &turn.role));
            }
        }
    }
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

/// Evidence-turn retrieval outcome for one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceScore {
    pub total: usize,
    pub retrieved: usize,
    /// 1-based rank of the first evidence turn in retrieval order.
    pub rank_first: Option<usize>,
    /// Sorted, for byte-identical repeat runs.
    pub missed: Vec<String>,
}

/// True when a retrieved key set can be compared against turn-shaped
/// evidence keys at all.
///
/// The read side has its own failure mode, independent of ingest: the
/// `Graph` retrieval path returns no raw hits, so `extract_keys` falls back
/// to parsing the formatted context, whose `--- Session <id>` blocks yield
/// bare session ids. Intersecting turn keys with session ids gives a silent,
/// total, fabricated 0/N. An empty retrieved set is a genuine zero and is
/// scored; a non-empty set with no turn-shaped key in it is refused.
fn retrieved_keys_are_turn_shaped(retrieved_keys: &[String]) -> bool {
    retrieved_keys.is_empty() || retrieved_keys.iter().any(|k| k.contains(":turn:"))
}

/// Score the evidence set against a retrieval order.
///
/// `None` = undefined, never 0 — see [`retrieved_keys_are_turn_shaped`].
pub fn score_evidence(
    evidence: &BTreeSet<String>,
    retrieved_keys: &[String],
) -> Option<EvidenceScore> {
    if !retrieved_keys_are_turn_shaped(retrieved_keys) {
        return None;
    }
    let got: HashSet<&str> = retrieved_keys.iter().map(String::as_str).collect();
    let retrieved = evidence.iter().filter(|k| got.contains(k.as_str())).count();
    let rank_first = retrieved_keys
        .iter()
        .position(|k| evidence.contains(k))
        .map(|p| p + 1);
    let missed: Vec<String> = evidence
        .iter()
        .filter(|k| !got.contains(k.as_str()))
        .cloned()
        .collect();
    Some(EvidenceScore {
        total: evidence.len(),
        retrieved,
        rank_first,
        missed,
    })
}

/// Return true when a key belongs to an answer session.
fn is_answer_key(key: &str) -> bool {
    key.split(':')
        .next()
        .map(|sid| sid.starts_with("answer_"))
        .unwrap_or(false)
}

/// Count answer-session turns and distinct answer sessions in the haystack.
///
/// The first return value is the diluted denominator R15 named: every turn
/// of an evidence session, not the evidence turns.
fn answer_totals(question: &Question, strategy: IngestStrategy) -> (usize, usize) {
    let mut keys = 0usize;
    let mut sessions = 0usize;
    for (idx, session) in question.haystack_sessions.iter().enumerate() {
        let sid = question
            .haystack_session_ids
            .get(idx)
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        if !sid.starts_with("answer_") {
            continue;
        }
        sessions += 1;
        keys += match strategy {
            IngestStrategy::PerTurn => session.len(),
            IngestStrategy::PerSession => 1,
        };
    }
    (keys, sessions)
}

/// Extract retrieved keys, preferring raw hits; falls back to parsing the
/// formatted context (same logic as eval.rs uses for the Graph path).
fn extract_keys(raw_hits: &[spectral_ingest::MemoryHit], memories: &[String]) -> Vec<String> {
    if !raw_hits.is_empty() {
        return raw_hits.iter().map(|h| h.key.clone()).collect();
    }
    memories
        .iter()
        .filter_map(|m| {
            if m.starts_with("--- Session ") {
                let rest = m.strip_prefix("--- Session ")?;
                let id = rest.split(' ').next()?;
                return Some(id.to_string());
            }
            let first_close = m.find("] ")?;
            let after_first = &m[first_close + 2..];
            let second_close = after_first.find("] ")?;
            let key_and_content = &after_first[second_close + 2..];
            key_and_content.split(": ").next().map(|k| k.to_string())
        })
        .collect()
}

/// Run the oracle over the dataset. Zero LLM calls.
pub fn run_oracle(config: &OracleConfig) -> Result<Vec<OracleRow>> {
    let ds = crate::dataset::load_dataset(&config.dataset_path)?;

    let expansion_cache: HashMap<String, String> = match &config.expansion_cache {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading expansion cache {}", path.display()))?;
            serde_json::from_str(&raw)?
        }
        None => HashMap::new(),
    };

    let mut questions: Vec<&Question> = ds.iter().collect();
    if let Some(ref cats) = config.categories {
        let allowed: std::collections::HashSet<&str> = cats.iter().map(|c| c.as_str()).collect();
        questions.retain(|q| allowed.contains(q.question_type.as_str()));
    }
    if let Some(ref qid) = config.question_id {
        questions.retain(|q| q.question_id == *qid);
    }
    if let Some(max) = config.max_questions {
        questions.truncate(max);
    }

    std::fs::create_dir_all(&config.work_dir)?;
    let mut out = std::io::BufWriter::new(
        std::fs::File::create(&config.output)
            .with_context(|| format!("creating {}", config.output.display()))?,
    );

    let pb = indicatif::ProgressBar::new(questions.len() as u64);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut rows = Vec::with_capacity(questions.len());
    let mut labelled_questions = 0usize;
    let mut shape_refusals = 0usize;
    for question in &questions {
        let brain_dir = config
            .work_dir
            .join(format!("brain_{}", question.question_id));

        let reused = config.reuse_brains && brain_dir.join("memory.db").exists();
        let brain = if reused {
            open_existing_brain(&brain_dir)?
        } else {
            let _ = std::fs::remove_dir_all(&brain_dir);
            ingest::ingest_question(question, &brain_dir, config.ingest_strategy)?
        };

        let retrieval_query = expansion_cache
            .get(&question.question_id)
            .cloned()
            .unwrap_or_else(|| question.question.clone());

        // Mirror eval_single routing exactly: classify on the ORIGINAL question.
        let qtype = retrieval::classify_question(&question.question);
        let effective_path = config
            .retrieval_path_override
            .unwrap_or_else(|| qtype.retrieval_path());

        let question_date = question.question_date.as_deref();
        let t = std::time::Instant::now();
        let (memories, raw_hits) = match effective_path {
            RetrievalPath::TopkFts => {
                let (formatted, hits) = retrieval::retrieve_topk_fts(
                    &brain,
                    &retrieval_query,
                    &config.retrieval,
                    question_date,
                )?;
                (formatted, hits)
            }
            RetrievalPath::Tact => {
                let result = brain.recall_local(&retrieval_query)?;
                let hits: Vec<_> = result
                    .memory_hits
                    .into_iter()
                    .take(config.retrieval.max_results)
                    .collect();
                let formatted: Vec<String> = hits.iter().map(retrieval::format_hit).collect();
                (formatted, hits)
            }
            RetrievalPath::Graph => {
                let formatted =
                    retrieval::retrieve_graph(&brain, &retrieval_query, &config.retrieval)?;
                (formatted, Vec::new())
            }
            RetrievalPath::Cascade => {
                let (formatted, hits, _telemetry) = retrieval::retrieve_cascade(
                    &brain,
                    &retrieval_query,
                    &config.retrieval,
                    question_date,
                )?;
                (formatted, hits)
            }
        };
        let retrieval_wall_ms = t.elapsed().as_millis() as u64;

        let retrieved_keys = extract_keys(&raw_hits, &memories);
        let (answer_session_turns_total, answer_sessions_total) =
            answer_totals(question, config.ingest_strategy);

        // R15: the real, dataset-labelled metric. `None` propagates as
        // "undefined" all the way to the summary; it is never coerced to 0.
        let evidence = evidence_keys(question, config.ingest_strategy);
        if evidence.is_some() {
            labelled_questions += 1;
        }
        let ev = evidence
            .as_ref()
            .and_then(|e| score_evidence(e, &retrieved_keys));
        if evidence.is_some() && ev.is_none() {
            shape_refusals += 1;
        }

        let answer_session_turns_retrieved =
            retrieved_keys.iter().filter(|k| is_answer_key(k)).count();
        let rank_first_answer_session_turn = retrieved_keys
            .iter()
            .position(|k| is_answer_key(k))
            .map(|p| p + 1);

        let hit_sessions: std::collections::HashSet<&str> = retrieved_keys
            .iter()
            .filter(|k| is_answer_key(k))
            .filter_map(|k| k.split(':').next())
            .collect();
        let answer_sessions_hit = hit_sessions.len();

        let actor_context = memories.join("\n");
        let context_hash = blake3::hash(actor_context.as_bytes()).to_hex().to_string();

        let row = OracleRow {
            label: config.label.clone(),
            question_id: question.question_id.clone(),
            category: question.question_type.clone(),
            shape: format!("{qtype:?}"),
            retrieval_path: format!("{effective_path:?}"),
            n_retrieved: retrieved_keys.len(),
            answer_session_turns_total,
            answer_session_turns_retrieved,
            answer_sessions_total,
            answer_sessions_hit,
            rank_first_answer_session_turn,
            evidence_turns_total: ev.as_ref().map(|e| e.total),
            evidence_turns_retrieved: ev.as_ref().map(|e| e.retrieved),
            rank_first_evidence_turn: ev.as_ref().and_then(|e| e.rank_first),
            evidence_keys_missed: ev.as_ref().map(|e| e.missed.clone()).unwrap_or_default(),
            context_chars: actor_context.len(),
            context_tokens_est: actor_context.len() / 4,
            context_hash,
            retrieval_wall_ms,
            retrieved_keys,
        };
        serde_json::to_writer(&mut out, &row)?;
        out.write_all(b"\n")?;
        rows.push(row);

        if !config.keep_brains {
            let _ = std::fs::remove_dir_all(&brain_dir);
        }
        pb.inc(1);
    }
    pb.finish_and_clear();
    out.flush()?;

    if labelled_questions == 0 {
        eprintln!(
            "warning: dataset carries no `has_answer` labels (LoCoMo-converted sets); \
             ev-* metrics are UNAVAILABLE, not zero."
        );
    }
    if shape_refusals > 0 {
        eprintln!(
            "warning: {shape_refusals} labelled question(s) returned a retrieved key set with no \
             turn-shaped key (e.g. the Graph path, which yields session ids). Evidence recall is \
             recorded as undefined for those rows, not 0."
        );
    }

    Ok(rows)
}

/// Aggregate stats for a set of oracle rows.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct OracleSummary {
    pub n: usize,
    pub mean_session_recall: f64,

    // ── Diluted legacy metric. Kept for continuity with archived runs only. ──
    /// Mean fraction of *answer-session* turns retrieved. This is the field
    /// published as "key-recall" before 2026-08-07. It is not evidence recall.
    pub mean_answer_session_turn_coverage: f64,
    pub zero_answer_session_turn_questions: usize,
    pub mean_rank_first_answer_session_turn: f64,

    // ── R15 evidence metrics ──
    /// Rows with a defined evidence metric.
    pub n_evidence_labeled: usize,
    /// Rows where the metric is undefined (no labels, non-PerTurn ingest, or
    /// a retrieved key set that is not turn-shaped). Excluded from every
    /// evidence figure below — they are not zeroes.
    pub n_evidence_unlabeled: usize,
    /// Micro denominator: labelled evidence turns across labelled rows.
    pub evidence_turns_total: usize,
    /// Micro numerator.
    pub evidence_turns_retrieved: usize,
    /// `evidence_turns_retrieved / evidence_turns_total`; 0.0 iff denominator 0.
    pub micro_evidence_recall: f64,
    /// Mean per-question recall over LABELLED rows only.
    pub macro_evidence_recall: f64,
    pub zero_evidence_questions: usize,
    pub full_evidence_questions: usize,
    pub mean_rank_first_evidence: f64,

    pub mean_tokens: f64,
    pub p95_tokens: usize,
}

/// Summarize rows (call with all rows, or a per-category slice).
///
/// **The load-bearing rule:** a row whose `evidence_turns_total` is `None`
/// increments only `n_evidence_unlabeled`. It never enters the macro mean,
/// never counts as a zero-evidence question, and never enters the micro
/// denominator. Counting the 21 LongMemEval `_abs` questions as 0 would drag
/// macro recall 90.5% → 86.7% and inflate zero-evidence 27 → 48: a
/// fabricated regression. Do not `unwrap_or(0)` these fields.
pub fn summarize(rows: &[&OracleRow]) -> OracleSummary {
    if rows.is_empty() {
        return OracleSummary::default();
    }
    let n = rows.len();
    let mut session_recall_sum = 0.0;
    let mut coverage_sum = 0.0;
    let mut zero = 0usize;
    let mut rank_sum = 0.0;
    let mut rank_n = 0usize;
    let mut tokens: Vec<usize> = Vec::with_capacity(n);

    let mut n_labeled = 0usize;
    let mut ev_total = 0usize;
    let mut ev_retrieved = 0usize;
    let mut macro_sum = 0.0;
    let mut ev_zero = 0usize;
    let mut ev_full = 0usize;
    let mut ev_rank_sum = 0.0;
    let mut ev_rank_n = 0usize;

    for r in rows {
        if r.answer_sessions_total > 0 {
            session_recall_sum += r.answer_sessions_hit as f64 / r.answer_sessions_total as f64;
        }
        if r.answer_session_turns_total > 0 {
            coverage_sum +=
                r.answer_session_turns_retrieved as f64 / r.answer_session_turns_total as f64;
        }
        if r.answer_session_turns_retrieved == 0 {
            zero += 1;
        }
        if let Some(rank) = r.rank_first_answer_session_turn {
            rank_sum += rank as f64;
            rank_n += 1;
        }
        tokens.push(r.context_tokens_est);

        if let (Some(total), Some(retrieved)) = (r.evidence_turns_total, r.evidence_turns_retrieved)
        {
            n_labeled += 1;
            ev_total += total;
            ev_retrieved += retrieved;
            if total > 0 {
                macro_sum += retrieved as f64 / total as f64;
            }
            if retrieved == 0 {
                ev_zero += 1;
            }
            if retrieved == total {
                ev_full += 1;
            }
            if let Some(rank) = r.rank_first_evidence_turn {
                ev_rank_sum += rank as f64;
                ev_rank_n += 1;
            }
        }
    }
    tokens.sort_unstable();
    let p95_tokens = tokens[((n as f64 * 0.95) as usize).min(n - 1)];
    OracleSummary {
        n,
        mean_session_recall: session_recall_sum / n as f64,
        mean_answer_session_turn_coverage: coverage_sum / n as f64,
        zero_answer_session_turn_questions: zero,
        mean_rank_first_answer_session_turn: if rank_n > 0 {
            rank_sum / rank_n as f64
        } else {
            0.0
        },
        n_evidence_labeled: n_labeled,
        n_evidence_unlabeled: n - n_labeled,
        evidence_turns_total: ev_total,
        evidence_turns_retrieved: ev_retrieved,
        micro_evidence_recall: if ev_total > 0 {
            ev_retrieved as f64 / ev_total as f64
        } else {
            0.0
        },
        macro_evidence_recall: if n_labeled > 0 {
            macro_sum / n_labeled as f64
        } else {
            0.0
        },
        zero_evidence_questions: ev_zero,
        full_evidence_questions: ev_full,
        mean_rank_first_evidence: if ev_rank_n > 0 {
            ev_rank_sum / ev_rank_n as f64
        } else {
            0.0
        },
        mean_tokens: tokens.iter().sum::<usize>() as f64 / n as f64,
        p95_tokens,
    }
}

/// Format one summary row of the per-category table.
fn summary_line(name: &str, s: &OracleSummary) -> String {
    let (ev_rec, ev_mic, ev_zero, ev_rank) = if s.n_evidence_labeled == 0 {
        (
            "n/a".to_string(),
            "n/a".to_string(),
            "n/a".to_string(),
            "n/a".to_string(),
        )
    } else {
        (
            format!("{:.1}%", s.macro_evidence_recall * 100.0),
            format!("{}/{}", s.evidence_turns_retrieved, s.evidence_turns_total),
            s.zero_evidence_questions.to_string(),
            format!("{:.1}", s.mean_rank_first_evidence),
        )
    };
    format!(
        "{:<28} {:>4} {:>8.1}% {:>8} {:>10} {:>8} {:>8.1}% {:>8} {:>7.1} {:>9} {:>8.0} {:>8}",
        name,
        s.n,
        s.mean_session_recall * 100.0,
        ev_rec,
        ev_mic,
        ev_zero,
        s.mean_answer_session_turn_coverage * 100.0,
        s.zero_answer_session_turn_questions,
        s.mean_rank_first_answer_session_turn,
        ev_rank,
        s.mean_tokens,
        s.p95_tokens
    )
}

/// Print a per-category summary table to stderr.
pub fn print_summary(rows: &[OracleRow]) {
    let all: Vec<&OracleRow> = rows.iter().collect();
    let overall = summarize(&all);
    eprintln!("\n=== ORACLE SUMMARY ({} questions) ===", overall.n);
    eprintln!(
        "{:<28} {:>4} {:>9} {:>8} {:>10} {:>8} {:>9} {:>8} {:>7} {:>9} {:>8} {:>8}",
        "category",
        "n",
        "sess-rec",
        "ev-rec",
        "ev-mic",
        "ev-zero",
        "as-cov",
        "as-zero",
        "rank1",
        "ev-rank1",
        "tok-mean",
        "tok-p95"
    );

    let mut categories: Vec<String> = rows.iter().map(|r| r.category.clone()).collect();
    categories.sort();
    categories.dedup();
    for cat in &categories {
        let slice: Vec<&OracleRow> = rows.iter().filter(|r| &r.category == cat).collect();
        eprintln!("{}", summary_line(cat, &summarize(&slice)));
    }
    eprintln!("{}", summary_line("TOTAL", &overall));

    eprintln!(
        "\nevidence labels: {}/{} questions scored; {} excluded (no `has_answer` label — the \
         LongMemEval `_abs` abstention questions and every LoCoMo-converted set — or a \
         retrieved key set that is not turn-shaped). Excluded rows are UNDEFINED, not 0.",
        overall.n_evidence_labeled, overall.n, overall.n_evidence_unlabeled
    );
    eprintln!(
        "as-cov/as-zero = answer-SESSION turn coverage (the old \"key-recall\"); it is ~12x \
         diluted and is NOT evidence recall — see R15 / \
         turn-level-evidence-recall-2026-08-07.md."
    );
}

/// Load oracle rows from a JSONL file.
pub fn load_rows(path: &Path) -> Result<Vec<OracleRow>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading oracle rows {}", path.display()))?;
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(Into::into))
        .collect()
}

/// One backfilled evidence row, written to the sidecar JSONL.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvidenceSidecarRow {
    pub question_id: String,
    pub category: String,
    pub evidence_turns_total: Option<usize>,
    pub evidence_turns_retrieved: Option<usize>,
    pub rank_first_evidence_turn: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_keys_missed: Vec<String>,
}

/// Rescore archived oracle rows against the dataset's `has_answer` labels,
/// **without re-running retrieval** — only `retrieved_keys` is consulted, and
/// it is present in every archived row file.
///
/// Returns rows carrying the backfilled evidence fields (all other fields
/// copied from the input). The caller decides what to do with them; nothing
/// here rewrites the input file (append-only: evidence is never erased).
pub fn backfill_evidence(
    questions: &[Question],
    rows: &[OracleRow],
    strategy: IngestStrategy,
) -> Vec<OracleRow> {
    let by_id: HashMap<&str, &Question> = questions
        .iter()
        .map(|q| (q.question_id.as_str(), q))
        .collect();
    rows.iter()
        .map(|r| {
            let mut out = r.clone();
            let ev = by_id
                .get(r.question_id.as_str())
                .and_then(|q| evidence_keys(q, strategy))
                .and_then(|e| score_evidence(&e, &r.retrieved_keys));
            out.evidence_turns_total = ev.as_ref().map(|e| e.total);
            out.evidence_turns_retrieved = ev.as_ref().map(|e| e.retrieved);
            out.rank_first_evidence_turn = ev.as_ref().and_then(|e| e.rank_first);
            out.evidence_keys_missed = ev.map(|e| e.missed).unwrap_or_default();
            out
        })
        .collect()
}

/// Sidecar view of backfilled rows, for writing next to (never over) the
/// archived input.
pub fn evidence_sidecar(rows: &[OracleRow]) -> Vec<EvidenceSidecarRow> {
    rows.iter()
        .map(|r| EvidenceSidecarRow {
            question_id: r.question_id.clone(),
            category: r.category.clone(),
            evidence_turns_total: r.evidence_turns_total,
            evidence_turns_retrieved: r.evidence_turns_retrieved,
            rank_first_evidence_turn: r.rank_first_evidence_turn,
            evidence_keys_missed: r.evidence_keys_missed.clone(),
        })
        .collect()
}

/// Paired diff between two oracle runs. Joins on question_id.
pub fn print_diff(baseline: &[OracleRow], candidate: &[OracleRow]) {
    let base: HashMap<&str, &OracleRow> = baseline
        .iter()
        .map(|r| (r.question_id.as_str(), r))
        .collect();

    let mut contexts_changed = 0usize;
    let mut sessions_improved: Vec<&str> = Vec::new();
    let mut sessions_regressed: Vec<&str> = Vec::new();
    let mut keys_delta_sum = 0i64;
    let mut tokens_delta_sum = 0i64;
    let mut zero_fixed: Vec<&str> = Vec::new();
    let mut zero_introduced: Vec<&str> = Vec::new();
    let mut joined = 0usize;

    let mut ev_joined = 0usize;
    let mut ev_base_total = 0usize;
    let mut ev_base_retrieved = 0usize;
    let mut ev_cand_total = 0usize;
    let mut ev_cand_retrieved = 0usize;
    let mut ev_delta_sum = 0i64;
    let mut ev_zero_fixed: Vec<&str> = Vec::new();
    let mut ev_zero_introduced: Vec<&str> = Vec::new();

    for cand in candidate {
        let Some(b) = base.get(cand.question_id.as_str()) else {
            continue;
        };
        joined += 1;
        if b.context_hash != cand.context_hash {
            contexts_changed += 1;
        }
        match cand.answer_sessions_hit.cmp(&b.answer_sessions_hit) {
            std::cmp::Ordering::Greater => sessions_improved.push(&cand.question_id),
            std::cmp::Ordering::Less => sessions_regressed.push(&cand.question_id),
            std::cmp::Ordering::Equal => {}
        }
        if b.answer_session_turns_retrieved == 0 && cand.answer_session_turns_retrieved > 0 {
            zero_fixed.push(&cand.question_id);
        }
        if b.answer_session_turns_retrieved > 0 && cand.answer_session_turns_retrieved == 0 {
            zero_introduced.push(&cand.question_id);
        }
        keys_delta_sum +=
            cand.answer_session_turns_retrieved as i64 - b.answer_session_turns_retrieved as i64;
        tokens_delta_sum += cand.context_tokens_est as i64 - b.context_tokens_est as i64;

        // R15: the honest gate metrics. Only rows where BOTH arms have a
        // defined evidence metric can be compared at all.
        if let (Some(bt), Some(br), Some(ct), Some(cr)) = (
            b.evidence_turns_total,
            b.evidence_turns_retrieved,
            cand.evidence_turns_total,
            cand.evidence_turns_retrieved,
        ) {
            ev_joined += 1;
            ev_base_total += bt;
            ev_base_retrieved += br;
            ev_cand_total += ct;
            ev_cand_retrieved += cr;
            ev_delta_sum += cr as i64 - br as i64;
            if br == 0 && cr > 0 {
                ev_zero_fixed.push(&cand.question_id);
            }
            if br > 0 && cr == 0 {
                ev_zero_introduced.push(&cand.question_id);
            }
        }
    }

    eprintln!("\n=== ORACLE DIFF (candidate vs baseline, {joined} joined) ===");
    eprintln!("contexts changed:            {contexts_changed} / {joined}");
    eprintln!(
        "session-recall improved:     {} {:?}",
        sessions_improved.len(),
        sessions_improved
    );
    eprintln!(
        "session-recall regressed:    {} {:?}",
        sessions_regressed.len(),
        sessions_regressed
    );
    eprintln!(
        "zero-answer-session-turn fixed:      {} {:?}",
        zero_fixed.len(),
        zero_fixed
    );
    eprintln!(
        "zero-answer-session-turn introduced: {} {:?}",
        zero_introduced.len(),
        zero_introduced
    );
    eprintln!("net answer-session-turns delta:      {keys_delta_sum:+}");
    eprintln!("  (the three lines above are the ~12x-diluted legacy metric, kept only for");
    eprintln!("   continuity with archived diffs — they are NOT evidence recall.)");

    if ev_joined == 0 {
        eprintln!(
            "\nevidence metric:             UNAVAILABLE on both arms (no `has_answer` labels, \
             or rows predate R15). Delta unknown — not zero."
        );
    } else {
        eprintln!("\n--- R15 evidence-turn metrics ({ev_joined} rows defined on both arms) ---");
        eprintln!(
            "zero-EVIDENCE fixed:         {} {:?}",
            ev_zero_fixed.len(),
            ev_zero_fixed
        );
        eprintln!(
            "zero-EVIDENCE introduced:    {} {:?}",
            ev_zero_introduced.len(),
            ev_zero_introduced
        );
        eprintln!("net evidence-turns delta:    {ev_delta_sum:+}");
        let bp = if ev_base_total > 0 {
            ev_base_retrieved as f64 / ev_base_total as f64 * 100.0
        } else {
            0.0
        };
        let cp = if ev_cand_total > 0 {
            ev_cand_retrieved as f64 / ev_cand_total as f64 * 100.0
        } else {
            0.0
        };
        eprintln!(
            "evidence-turn recall:        {bp:.1}% ({ev_base_retrieved}/{ev_base_total}) -> \
             {cp:.1}% ({ev_cand_retrieved}/{ev_cand_total}) (micro)"
        );
    }

    eprintln!(
        "\nmean tokens delta:           {:+.0}",
        tokens_delta_sum as f64 / joined.max(1) as f64
    );
    eprintln!(
        "\nTier-1 candidate set (actor replay needed): the {contexts_changed} changed-context questions."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Turn;

    fn answer_question() -> Question {
        Question {
            question_id: "q-oracle".into(),
            question_type: "single-session-user".into(),
            question: "What color is the sky?".into(),
            answer: serde_json::Value::String("Blue".into()),
            question_date: Some("2023/05/30 (Tue) 23:40".into()),
            haystack_sessions: vec![
                vec![
                    Turn {
                        role: "user".into(),
                        content: "The sky is blue today and I love it.".into(),
                        // The one labelled evidence turn.
                        has_answer: Some(true),
                        speaker: None,
                    },
                    Turn {
                        role: "assistant".into(),
                        content: "That sounds lovely! Blue skies are wonderful.".into(),
                        // Explicit `false` occurs in the real dataset and is
                        // NOT evidence.
                        has_answer: Some(false),
                        speaker: None,
                    },
                ],
                vec![Turn {
                    role: "user".into(),
                    content: "I ate pasta for dinner yesterday evening.".into(),
                    has_answer: None,
                    speaker: None,
                }],
            ],
            haystack_session_ids: vec!["answer_abc_1".into(), "noise_1".into()],
            haystack_dates: vec![
                "2023/02/15 (Wed) 23:50".into(),
                "2023/02/16 (Thu) 10:00".into(),
            ],
        }
    }

    #[test]
    fn is_answer_key_matches_convention() {
        assert!(is_answer_key("answer_abc_1:turn:0:user"));
        assert!(!is_answer_key("noise_1:turn:0:user"));
        assert!(!is_answer_key("unknown"));
    }

    #[test]
    fn answer_totals_per_turn_counts_turns() {
        let q = answer_question();
        let (keys, sessions) = answer_totals(&q, IngestStrategy::PerTurn);
        assert_eq!(keys, 2);
        assert_eq!(sessions, 1);
    }

    #[test]
    fn answer_totals_per_session_counts_sessions() {
        let q = answer_question();
        let (keys, sessions) = answer_totals(&q, IngestStrategy::PerSession);
        assert_eq!(keys, 1);
        assert_eq!(sessions, 1);
    }

    #[test]
    fn oracle_end_to_end_finds_answer_and_evidence_turns() {
        let dir = tempfile::tempdir().unwrap();
        let dataset_path = dir.path().join("ds.json");
        std::fs::write(
            &dataset_path,
            serde_json::to_string(&vec![answer_question()]).unwrap(),
        )
        .unwrap();

        let config = OracleConfig {
            dataset_path,
            work_dir: dir.path().join("work"),
            output: dir.path().join("oracle.jsonl"),
            max_questions: None,
            categories: None,
            question_id: None,
            ingest_strategy: IngestStrategy::PerTurn,
            retrieval: RetrievalConfig::default(),
            retrieval_path_override: None,
            reuse_brains: false,
            keep_brains: true,
            label: "test".into(),
            expansion_cache: None,
        };

        let rows = run_oracle(&config).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.answer_session_turns_total, 2);
        assert!(
            row.answer_session_turns_retrieved > 0,
            "sky question should retrieve the answer session; got keys {:?}",
            row.retrieved_keys
        );
        assert_eq!(row.answer_sessions_total, 1);
        assert_eq!(row.answer_sessions_hit, 1);
        assert!(row.rank_first_answer_session_turn.is_some());
        assert!(!row.context_hash.is_empty());

        // R15: exactly one labelled evidence turn, and it is retrieved.
        // Note the denominators differ: 2 answer-session turns, 1 evidence
        // turn. That gap is the whole point of the metric.
        assert_eq!(row.evidence_turns_total, Some(1));
        assert_eq!(
            row.evidence_turns_retrieved,
            Some(1),
            "evidence key missed: {:?} (retrieved {:?})",
            row.evidence_keys_missed,
            row.retrieved_keys
        );
        assert!(row.rank_first_evidence_turn.is_some());
        assert!(row.evidence_keys_missed.is_empty());

        // Rows round-trip from disk.
        let loaded = load_rows(&config.output).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].question_id, row.question_id);
    }

    #[test]
    fn reuse_brains_produces_identical_context_hash() {
        let dir = tempfile::tempdir().unwrap();
        let dataset_path = dir.path().join("ds.json");
        std::fs::write(
            &dataset_path,
            serde_json::to_string(&vec![answer_question()]).unwrap(),
        )
        .unwrap();

        let mut config = OracleConfig {
            dataset_path,
            work_dir: dir.path().join("work"),
            output: dir.path().join("oracle-1.jsonl"),
            max_questions: None,
            categories: None,
            question_id: None,
            ingest_strategy: IngestStrategy::PerTurn,
            retrieval: RetrievalConfig::default(),
            retrieval_path_override: None,
            reuse_brains: false,
            keep_brains: true,
            label: "first".into(),
            expansion_cache: None,
        };
        let first = run_oracle(&config).unwrap();

        config.output = dir.path().join("oracle-2.jsonl");
        config.reuse_brains = true;
        config.label = "second".into();
        let second = run_oracle(&config).unwrap();

        assert_eq!(first[0].context_hash, second[0].context_hash);
        assert_eq!(
            first[0].retrieved_keys, second[0].retrieved_keys,
            "reused brain must produce identical retrieval"
        );
    }

    #[test]
    fn summarize_handles_rows() {
        let row = OracleRow {
            label: "t".into(),
            question_id: "q1".into(),
            category: "multi-session".into(),
            shape: "Factual".into(),
            retrieval_path: "Cascade".into(),
            n_retrieved: 10,
            answer_session_turns_total: 4,
            answer_session_turns_retrieved: 2,
            answer_sessions_total: 2,
            answer_sessions_hit: 1,
            rank_first_answer_session_turn: Some(3),
            evidence_turns_total: Some(2),
            evidence_turns_retrieved: Some(1),
            rank_first_evidence_turn: Some(3),
            evidence_keys_missed: vec![],
            context_chars: 4000,
            context_tokens_est: 1000,
            context_hash: "abc".into(),
            retrieval_wall_ms: 5,
            retrieved_keys: vec![],
        };
        let rows = [&row];
        let s = summarize(&rows);
        assert_eq!(s.n, 1);
        assert!((s.mean_session_recall - 0.5).abs() < 1e-9);
        assert!((s.mean_answer_session_turn_coverage - 0.5).abs() < 1e-9);
        assert_eq!(s.zero_answer_session_turn_questions, 0);
        assert_eq!(s.n_evidence_labeled, 1);
        assert_eq!(s.evidence_turns_total, 2);
        assert_eq!(s.evidence_turns_retrieved, 1);
        assert!((s.micro_evidence_recall - 0.5).abs() < 1e-9);
        assert_eq!(s.p95_tokens, 1000);
    }

    // ───────────────────────── R15 evidence metric ─────────────────────────

    fn row_with_keys(id: &str, keys: Vec<&str>) -> OracleRow {
        OracleRow {
            label: "t".into(),
            question_id: id.into(),
            category: "multi-session".into(),
            shape: "Factual".into(),
            retrieval_path: "Cascade".into(),
            n_retrieved: keys.len(),
            answer_session_turns_total: 0,
            answer_session_turns_retrieved: 0,
            answer_sessions_total: 0,
            answer_sessions_hit: 0,
            rank_first_answer_session_turn: None,
            evidence_turns_total: None,
            evidence_turns_retrieved: None,
            rank_first_evidence_turn: None,
            evidence_keys_missed: vec![],
            context_chars: 0,
            context_tokens_est: 0,
            context_hash: "h".into(),
            retrieval_wall_ms: 0,
            retrieved_keys: keys.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn evidence_keys_are_ingestable_keys() {
        // Anti-drift: every key `evidence_keys` produces must be a key ingest
        // actually wrote. If the two formats ever diverge, evidence recall
        // silently reads 0% and looks like a retrieval catastrophe.
        let q = answer_question();
        let dir = tempfile::tempdir().unwrap();
        let brain = ingest::ingest_question(&q, dir.path(), IngestStrategy::PerTurn).unwrap();
        let stored: HashSet<String> = brain
            .list_all_memories(1000)
            .unwrap()
            .into_iter()
            .map(|m| m.key)
            .collect();
        let evidence = evidence_keys(&q, IngestStrategy::PerTurn).expect("fixture is labelled");
        assert!(!evidence.is_empty());
        for key in &evidence {
            assert!(
                stored.contains(key),
                "evidence key {key} was never ingested; stored = {stored:?}"
            );
        }
    }

    #[test]
    fn has_answer_false_and_absent_are_not_evidence() {
        let q = answer_question();
        let evidence = evidence_keys(&q, IngestStrategy::PerTurn).unwrap();
        // turn 0 is Some(true); turn 1 is Some(false); the noise session's
        // turn is None. Only the first counts.
        assert_eq!(
            evidence.iter().cloned().collect::<Vec<_>>(),
            vec!["answer_abc_1:turn:0:user".to_string()]
        );
    }

    #[test]
    fn unlabeled_question_yields_none() {
        let mut q = answer_question();
        for session in &mut q.haystack_sessions {
            for turn in session {
                turn.has_answer = None;
            }
        }
        assert!(
            evidence_keys(&q, IngestStrategy::PerTurn).is_none(),
            "an unlabelled question must be UNDEFINED, never Some(0)"
        );
    }

    #[test]
    fn per_session_strategy_refuses_the_turn_metric() {
        // Under PerSession every turn collapses to `{sid}:session`, so the
        // quantity would be evidence SESSIONS while the field name says
        // turns. Refuse rather than publish a false name. Shipped config is
        // PerTurn, so this only guards misuse.
        let q = answer_question();
        assert!(evidence_keys(&q, IngestStrategy::PerSession).is_none());
    }

    #[test]
    fn score_evidence_refuses_session_shaped_retrieved_keys() {
        // The read-side failure mode: the Graph path returns no raw hits, so
        // `extract_keys` falls back to parsing `--- Session <id>` blocks and
        // yields bare session ids. Intersecting those with turn keys would
        // report a fabricated 0/N.
        let evidence: BTreeSet<String> = ["answer_abc_1:turn:0:user".to_string()]
            .into_iter()
            .collect();
        let session_shaped = vec!["answer_abc_1".to_string(), "noise_1".to_string()];
        assert!(
            score_evidence(&evidence, &session_shaped).is_none(),
            "session-shaped retrieval must make the metric UNDEFINED, not 0/N"
        );

        // An empty retrieved set is a genuine zero and is still scored.
        let scored = score_evidence(&evidence, &[]).expect("empty retrieval is a real zero");
        assert_eq!(scored.retrieved, 0);
        assert_eq!(scored.total, 1);
        assert_eq!(scored.missed, vec!["answer_abc_1:turn:0:user".to_string()]);
    }

    #[test]
    fn score_evidence_ranks_and_sorts_deterministically() {
        let evidence: BTreeSet<String> = [
            "answer_a:turn:3:user".to_string(),
            "answer_a:turn:1:user".to_string(),
            "answer_a:turn:9:user".to_string(),
        ]
        .into_iter()
        .collect();
        let retrieved = vec![
            "noise:turn:0:user".to_string(),
            "answer_a:turn:3:user".to_string(),
            "noise:turn:1:user".to_string(),
        ];
        let s = score_evidence(&evidence, &retrieved).unwrap();
        assert_eq!(s.total, 3);
        assert_eq!(s.retrieved, 1);
        assert_eq!(s.rank_first, Some(2));
        assert_eq!(
            s.missed,
            vec![
                "answer_a:turn:1:user".to_string(),
                "answer_a:turn:9:user".to_string()
            ],
            "missed keys must be sorted for byte-identical repeat runs"
        );
    }

    #[test]
    fn summarize_excludes_unlabeled_from_both_means() {
        let mut labeled_full = row_with_keys("q1", vec![]);
        labeled_full.evidence_turns_total = Some(2);
        labeled_full.evidence_turns_retrieved = Some(2);
        let mut labeled_zero = row_with_keys("q2", vec![]);
        labeled_zero.evidence_turns_total = Some(2);
        labeled_zero.evidence_turns_retrieved = Some(0);
        let unlabeled = row_with_keys("q3_abs", vec![]);

        let rows = [&labeled_full, &labeled_zero, &unlabeled];
        let s = summarize(&rows);
        assert_eq!(s.n, 3);
        assert_eq!(s.n_evidence_labeled, 2);
        assert_eq!(s.n_evidence_unlabeled, 1);
        assert_eq!(s.evidence_turns_total, 4, "micro denominator excludes q3");
        assert_eq!(s.evidence_turns_retrieved, 2);
        assert!((s.micro_evidence_recall - 0.5).abs() < 1e-9);
        assert!(
            (s.macro_evidence_recall - 0.5).abs() < 1e-9,
            "macro must divide by 2 labelled rows, not 3"
        );
        assert_eq!(s.zero_evidence_questions, 1, "q3 is undefined, not zero");
        assert_eq!(s.full_evidence_questions, 1);
    }

    #[test]
    fn archived_rows_deserialize_via_alias() {
        // A verbatim line from ~/spectral-local-bench/r12-baseline.jsonl,
        // truncated in `retrieved_keys` only. Pre-rename field names.
        let line = r#"{"label":"r12-baseline","question_id":"q1","category":"multi-session","shape":"Factual","retrieval_path":"Cascade","n_retrieved":2,"answer_keys_total":37,"answer_keys_retrieved":6,"answer_sessions_total":2,"answer_sessions_hit":2,"rank_first_answer_key":1,"context_chars":100,"context_tokens_est":25,"context_hash":"abc","retrieval_wall_ms":9,"retrieved_keys":["answer_1:turn:0:user","noise:turn:0:user"]}"#;
        let row: OracleRow = serde_json::from_str(line).unwrap();
        assert_eq!(row.answer_session_turns_total, 37);
        assert_eq!(row.answer_session_turns_retrieved, 6);
        assert_eq!(row.rank_first_answer_session_turn, Some(1));
        // Archived rows predate the evidence metric: undefined, not zero.
        assert_eq!(row.evidence_turns_total, None);
        assert_eq!(row.evidence_turns_retrieved, None);
    }

    #[test]
    fn archived_row_file_still_loads() {
        // The committed fixture is verbatim archived JSONL with the old field
        // names. If `load_rows` ever stops parsing it, the archive is lost.
        let rows = load_rows(Path::new("tests/fixtures/r15/r12-rows-subset.jsonl"))
            .expect("archived r12 rows must still load through the serde aliases");
        assert_eq!(rows.len(), 45);
        assert!(rows.iter().all(|r| r.evidence_turns_total.is_none()));
        assert!(rows.iter().any(|r| r.answer_session_turns_total > 0));
    }

    /// ACCEPTANCE TEST for R15, running in CI on committed data.
    ///
    /// Rescores real archived shipped-config rows against real LongMemEval
    /// labels. The numbers below are the 45-question fixture subset, not the
    /// full corpus — see `tests/fixtures/r15/README.md`. The full-corpus
    /// figures are pinned by `evidence_recall_reproduces_r15_note`.
    #[test]
    fn evidence_recall_pinned_on_committed_fixture() {
        let questions =
            crate::dataset::load_dataset(Path::new("tests/fixtures/r15/dataset-subset.json"))
                .unwrap();
        let rows = load_rows(Path::new("tests/fixtures/r15/r12-rows-subset.jsonl")).unwrap();
        assert_eq!(questions.len(), 45);
        assert_eq!(rows.len(), 45);

        let backfilled = backfill_evidence(&questions, &rows, IngestStrategy::PerTurn);
        let refs: Vec<&OracleRow> = backfilled.iter().collect();
        let s = summarize(&refs);

        assert_eq!(s.n, 45);
        assert_eq!(s.n_evidence_labeled, 42);
        assert_eq!(
            s.n_evidence_unlabeled, 3,
            "the `_abs` questions are undefined"
        );
        assert_eq!(s.evidence_turns_total, 78);
        assert_eq!(s.evidence_turns_retrieved, 68);
        assert!(
            (s.micro_evidence_recall - 68.0 / 78.0).abs() < 1e-12,
            "micro = {}",
            s.micro_evidence_recall
        );
        assert!(
            (s.macro_evidence_recall - 0.917_460_317_460_317_5).abs() < 1e-9,
            "macro = {}",
            s.macro_evidence_recall
        );
        assert_eq!(s.zero_evidence_questions, 1);
        assert_eq!(s.full_evidence_questions, 35);

        // The preference category is where the defect concentrates: the one
        // zero-evidence question in the subset is a preference question.
        let pref: Vec<&OracleRow> = backfilled
            .iter()
            .filter(|r| r.category == "single-session-preference")
            .collect();
        let ps = summarize(&pref);
        assert_eq!(ps.n_evidence_labeled, 3);
        assert_eq!(ps.evidence_turns_total, 4);
        assert_eq!(ps.evidence_turns_retrieved, 3);
        assert_eq!(ps.zero_evidence_questions, 1);

        // Backfill is a pure function of (labels, retrieved_keys): running it
        // twice is byte-identical.
        let again = backfill_evidence(&questions, &rows, IngestStrategy::PerTurn);
        assert_eq!(
            serde_json::to_string(&evidence_sidecar(&backfilled)).unwrap(),
            serde_json::to_string(&evidence_sidecar(&again)).unwrap()
        );
    }

    /// Machine-local second check: the FULL corpus figures the R15 note was
    /// written from. Requires `~/spectral-local-bench`.
    ///
    /// `cargo test -p spectral-bench-accuracy -- --ignored evidence_recall_reproduces_r15_note`
    #[test]
    #[ignore = "requires ~/spectral-local-bench (dataset + archived r12 rows)"]
    fn evidence_recall_reproduces_r15_note() {
        let home = std::env::var("HOME").unwrap();
        let base = PathBuf::from(home).join("spectral-local-bench");
        let questions =
            crate::dataset::load_dataset(&base.join("longmemeval/longmemeval_s.json")).unwrap();
        let rows = load_rows(&base.join("r12-baseline.jsonl")).unwrap();

        let backfilled = backfill_evidence(&questions, &rows, IngestStrategy::PerTurn);
        let refs: Vec<&OracleRow> = backfilled.iter().collect();
        let s = summarize(&refs);

        assert_eq!(s.n, 500);
        assert_eq!(s.n_evidence_labeled, 479);
        assert_eq!(s.n_evidence_unlabeled, 21);
        assert_eq!(s.evidence_turns_total, 896);
        assert_eq!(s.evidence_turns_retrieved, 793);
        assert!((s.micro_evidence_recall - 0.885).abs() < 0.0005);
        assert!((s.macro_evidence_recall - 0.905).abs() < 0.0005);
        assert_eq!(s.zero_evidence_questions, 27);
        assert_eq!(s.full_evidence_questions, 409);

        let pref: Vec<&OracleRow> = backfilled
            .iter()
            .filter(|r| r.category == "single-session-preference")
            .collect();
        let ps = summarize(&pref);
        assert_eq!(ps.evidence_turns_total, 44);
        assert_eq!(ps.evidence_turns_retrieved, 29);
        assert_eq!(ps.zero_evidence_questions, 9);
    }
}
