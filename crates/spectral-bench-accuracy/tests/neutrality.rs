//! Runtime neutrality proof: exercises retrieval on 7 real LongMemEval_S
//! questions through the same question-type routing the bench uses under
//! `--use-cascade`, and outputs ordered hit keys for branch-vs-main comparison.
//!
//! The 7-question subset is selected deterministically from the full
//! LongMemEval_S dataset at test time (no pre-extracted fixture): the first
//! 5 questions by question_id sort that classify non-Temporal (→ cascade,
//! Path A) plus the first 2 temporal-reasoning questions (→ topk_fts,
//! Path B), giving both retrieval paths coverage. The plain first 5 would
//! NOT give 5×Path A: 001be529 ("How long did I wait…") classifies
//! Temporal, so it is skipped in favor of the next cascade-routed question.
//!
//! Dataset location: `$LONGMEMEVAL_S_JSON` if set, otherwise
//! `$HOME/spectral-local-bench/longmemeval/longmemeval_s.json`. If the
//! dataset is absent (e.g. CI), the test SKIPS with a message rather than
//! failing — it proves neutrality, it does not gate correctness.
//!
//! Run with:
//!   cargo test -p spectral-bench-accuracy --test neutrality -- --nocapture
//!
//! Neutrality criterion: identical hit keys in identical order on branch
//! vs main for the same questions.
//!
//! ## Why the original version sent all questions through topk_fts
//!
//! It called `retrieve_topk_fts` directly instead of using the bench's
//! `QuestionType::classify → retrieval_path()` routing, so non-temporal
//! questions bypassed cascade (Path A) and only exercised Path B.

use spectral_bench_accuracy::dataset::Question;
use spectral_bench_accuracy::ingest;
use spectral_bench_accuracy::retrieval::{self, QuestionType, RetrievalConfig};
use std::path::PathBuf;

/// Resolve the LongMemEval_S dataset path: env override, then the local
/// bench convention.
fn dataset_path() -> PathBuf {
    if let Ok(p) = std::env::var("LONGMEMEVAL_S_JSON") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join("spectral-local-bench/longmemeval/longmemeval_s.json")
}

/// Deterministically select the 7-question neutrality subset: the first 5
/// by question_id sort that classify non-Temporal (Path A: cascade), plus
/// the first 2 with dataset question_type == "temporal-reasoning" that
/// classify Temporal (Path B: topk_fts).
fn select_neutrality_questions(mut all: Vec<Question>) -> Vec<Question> {
    all.sort_by(|a, b| a.question_id.cmp(&b.question_id));

    let mut path_a = Vec::new();
    let mut path_b = Vec::new();
    for q in all {
        let temporal = QuestionType::classify(&q.question) == QuestionType::Temporal;
        if !temporal && path_a.len() < 5 {
            path_a.push(q);
        } else if temporal && q.question_type == "temporal-reasoning" && path_b.len() < 2 {
            path_b.push(q);
        }
        if path_a.len() == 5 && path_b.len() == 2 {
            break;
        }
    }
    path_a.extend(path_b);
    path_a
}

#[test]
fn neutrality_hit_keys() {
    let path = dataset_path();
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(e) => {
            // Dataset not present (e.g. CI) — skip, don't fail.
            println!(
                "SKIP neutrality_hit_keys: LongMemEval_S dataset not found at {} ({e}). \
                 Set LONGMEMEVAL_S_JSON to run this test.",
                path.display()
            );
            return;
        }
    };
    let all: Vec<Question> = serde_json::from_str(&data).unwrap();
    let questions = select_neutrality_questions(all);
    assert_eq!(questions.len(), 7, "expected 7 neutrality questions");

    let dir = tempfile::tempdir().unwrap();
    let config = RetrievalConfig { max_results: 40 };

    println!("=== NEUTRALITY PROOF: hit keys per question ===");

    let mut cascade_count = 0;
    let mut topk_fts_count = 0;

    for q in &questions {
        let brain_dir = dir.path().join(format!("brain_{}", q.question_id));
        let brain =
            ingest::ingest_question(q, &brain_dir, ingest::IngestStrategy::PerTurn).unwrap();

        let question_date = q.question_date.as_deref();
        let qtype = QuestionType::classify(&q.question);
        let effective_path = qtype.retrieval_path();

        // Route exactly as the bench does under --use-cascade:
        //   Temporal → topk_fts (Path B)
        //   Everything else → cascade (Path A)
        let keys: Vec<String> = match effective_path {
            retrieval::RetrievalPath::TopkFts => {
                topk_fts_count += 1;
                let (_formatted, hits) =
                    retrieval::retrieve_topk_fts(&brain, &q.question, &config, question_date)
                        .unwrap();
                hits.iter().map(|h| h.key.clone()).collect()
            }
            retrieval::RetrievalPath::Cascade => {
                cascade_count += 1;
                let (_formatted, hits, _telemetry) =
                    retrieval::retrieve_cascade(&brain, &q.question, &config, question_date)
                        .unwrap();
                hits.iter().map(|h| h.key.clone()).collect()
            }
            _ => unreachable!("question-type routing only produces TopkFts or Cascade"),
        };

        println!(
            "{}|{}|{:?}|{:?}|hits={}|keys={}",
            q.question_id,
            q.question_type,
            qtype,
            effective_path,
            keys.len(),
            serde_json::to_string(&keys).unwrap()
        );

        let _ = std::fs::remove_dir_all(&brain_dir);
    }

    assert_eq!(cascade_count, 5, "expected 5 questions on Path A (cascade)");
    assert_eq!(
        topk_fts_count, 2,
        "expected 2 questions on Path B (topk_fts)"
    );

    println!("=== END ===");
}
