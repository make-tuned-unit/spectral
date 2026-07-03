//! Tier-0 retrieval oracle: retrieval-only evaluation with zero LLM calls.
//!
//! Measures what the memory layer alone delivers — answer-key recall, rank of
//! first answer key, retrieved-context size, and a context hash for paired
//! diffing between configurations — without spending a token on actor or
//! judge. This is the gate every retrieval-side change must clear before any
//! paid bench run.
//!
//! Answer-bearing keys follow the LongMemEval convention: a haystack session
//! whose id starts with `answer_` is an evidence session, and every turn
//! ingested from it (`{session_id}:turn:{idx}:{role}`) counts as an answer key.

use crate::dataset::{Category, Question};
use crate::ingest::{self, IngestStrategy};
use crate::retrieval::{self, QuestionType, RetrievalConfig, RetrievalPath};
use anyhow::{Context, Result};
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use spectral_tact::TactConfig;
use std::collections::HashMap;
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
    /// Total answer-bearing keys in the haystack for this question.
    pub answer_keys_total: usize,
    /// Answer-bearing keys present in the retrieved set.
    pub answer_keys_retrieved: usize,
    /// Distinct answer sessions in the haystack.
    pub answer_sessions_total: usize,
    /// Answer sessions with at least one retrieved turn.
    pub answer_sessions_hit: usize,
    /// 1-based rank of the first answer key in retrieval order. None = miss.
    pub rank_first_answer_key: Option<usize>,
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
    Ok(Brain::open(BrainConfig {
        data_dir: brain_dir.to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: std::env::var("SPECTRAL_BENCH_SPECTROGRAM").is_ok(),
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: Some(TactConfig {
            max_results: 20,
            ..TactConfig::default()
        }),
    })?)
}

/// Return true when a key belongs to an answer session.
fn is_answer_key(key: &str) -> bool {
    key.split(':')
        .next()
        .map(|sid| sid.starts_with("answer_"))
        .unwrap_or(false)
}

/// Count answer-bearing keys and distinct answer sessions in the haystack.
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
        let qtype = QuestionType::classify(&question.question);
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
        let (answer_keys_total, answer_sessions_total) =
            answer_totals(question, config.ingest_strategy);

        let answer_keys_retrieved = retrieved_keys.iter().filter(|k| is_answer_key(k)).count();
        let rank_first_answer_key = retrieved_keys
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
            answer_keys_total,
            answer_keys_retrieved,
            answer_sessions_total,
            answer_sessions_hit,
            rank_first_answer_key,
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

    Ok(rows)
}

/// Aggregate stats for a set of oracle rows.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct OracleSummary {
    pub n: usize,
    pub mean_session_recall: f64,
    pub mean_key_recall: f64,
    pub zero_answer_key_questions: usize,
    pub mean_rank_first: f64,
    pub mean_tokens: f64,
    pub p95_tokens: usize,
}

/// Summarize rows (call with all rows, or a per-category slice).
pub fn summarize(rows: &[&OracleRow]) -> OracleSummary {
    if rows.is_empty() {
        return OracleSummary::default();
    }
    let n = rows.len();
    let mut session_recall_sum = 0.0;
    let mut key_recall_sum = 0.0;
    let mut zero = 0usize;
    let mut rank_sum = 0.0;
    let mut rank_n = 0usize;
    let mut tokens: Vec<usize> = Vec::with_capacity(n);
    for r in rows {
        if r.answer_sessions_total > 0 {
            session_recall_sum += r.answer_sessions_hit as f64 / r.answer_sessions_total as f64;
        }
        if r.answer_keys_total > 0 {
            key_recall_sum += r.answer_keys_retrieved as f64 / r.answer_keys_total as f64;
        }
        if r.answer_keys_retrieved == 0 {
            zero += 1;
        }
        if let Some(rank) = r.rank_first_answer_key {
            rank_sum += rank as f64;
            rank_n += 1;
        }
        tokens.push(r.context_tokens_est);
    }
    tokens.sort_unstable();
    let p95_tokens = tokens[((n as f64 * 0.95) as usize).min(n - 1)];
    OracleSummary {
        n,
        mean_session_recall: session_recall_sum / n as f64,
        mean_key_recall: key_recall_sum / n as f64,
        zero_answer_key_questions: zero,
        mean_rank_first: if rank_n > 0 {
            rank_sum / rank_n as f64
        } else {
            0.0
        },
        mean_tokens: tokens.iter().sum::<usize>() as f64 / n as f64,
        p95_tokens,
    }
}

/// Print a per-category summary table to stderr.
pub fn print_summary(rows: &[OracleRow]) {
    let all: Vec<&OracleRow> = rows.iter().collect();
    let overall = summarize(&all);
    eprintln!("\n=== ORACLE SUMMARY ({} questions) ===", overall.n);
    eprintln!(
        "{:<28} {:>4} {:>9} {:>9} {:>6} {:>7} {:>8} {:>8}",
        "category", "n", "sess-rec", "key-rec", "zero", "rank1", "tok-mean", "tok-p95"
    );

    let mut categories: Vec<String> = rows.iter().map(|r| r.category.clone()).collect();
    categories.sort();
    categories.dedup();
    for cat in &categories {
        let slice: Vec<&OracleRow> = rows.iter().filter(|r| &r.category == cat).collect();
        let s = summarize(&slice);
        eprintln!(
            "{:<28} {:>4} {:>8.1}% {:>8.1}% {:>6} {:>7.1} {:>8.0} {:>8}",
            cat,
            s.n,
            s.mean_session_recall * 100.0,
            s.mean_key_recall * 100.0,
            s.zero_answer_key_questions,
            s.mean_rank_first,
            s.mean_tokens,
            s.p95_tokens
        );
    }
    eprintln!(
        "{:<28} {:>4} {:>8.1}% {:>8.1}% {:>6} {:>7.1} {:>8.0} {:>8}",
        "TOTAL",
        overall.n,
        overall.mean_session_recall * 100.0,
        overall.mean_key_recall * 100.0,
        overall.zero_answer_key_questions,
        overall.mean_rank_first,
        overall.mean_tokens,
        overall.p95_tokens
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
        if b.answer_keys_retrieved == 0 && cand.answer_keys_retrieved > 0 {
            zero_fixed.push(&cand.question_id);
        }
        if b.answer_keys_retrieved > 0 && cand.answer_keys_retrieved == 0 {
            zero_introduced.push(&cand.question_id);
        }
        keys_delta_sum += cand.answer_keys_retrieved as i64 - b.answer_keys_retrieved as i64;
        tokens_delta_sum += cand.context_tokens_est as i64 - b.context_tokens_est as i64;
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
        "zero-answer-key fixed:       {} {:?}",
        zero_fixed.len(),
        zero_fixed
    );
    eprintln!(
        "zero-answer-key introduced:  {} {:?}",
        zero_introduced.len(),
        zero_introduced
    );
    eprintln!("net answer-keys delta:       {keys_delta_sum:+}");
    eprintln!(
        "mean tokens delta:           {:+.0}",
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
                    },
                    Turn {
                        role: "assistant".into(),
                        content: "That sounds lovely! Blue skies are wonderful.".into(),
                    },
                ],
                vec![Turn {
                    role: "user".into(),
                    content: "I ate pasta for dinner yesterday evening.".into(),
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
    fn oracle_end_to_end_finds_answer_keys() {
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
        assert_eq!(row.answer_keys_total, 2);
        assert!(
            row.answer_keys_retrieved > 0,
            "sky question should retrieve the answer session; got keys {:?}",
            row.retrieved_keys
        );
        assert_eq!(row.answer_sessions_total, 1);
        assert_eq!(row.answer_sessions_hit, 1);
        assert!(row.rank_first_answer_key.is_some());
        assert!(!row.context_hash.is_empty());

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
            answer_keys_total: 4,
            answer_keys_retrieved: 2,
            answer_sessions_total: 2,
            answer_sessions_hit: 1,
            rank_first_answer_key: Some(3),
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
        assert!((s.mean_key_recall - 0.5).abs() < 1e-9);
        assert_eq!(s.zero_answer_key_questions, 0);
        assert_eq!(s.p95_tokens, 1000);
    }
}
