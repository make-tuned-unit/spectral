//! Runtime neutrality proof: exercises retrieval on 7 real LongMemEval_S
//! questions through the same question-type routing the bench uses under
//! `--use-cascade`, and outputs ordered hit keys for branch-vs-main comparison.
//!
//! Questions are the first 5 by question_id sort (non-temporal → cascade,
//! Path A) plus the first 2 temporal-reasoning questions (→ topk_fts,
//! Path B), giving both retrieval paths coverage.
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

use spectral_bench_accuracy::ingest;
use spectral_bench_accuracy::retrieval::{self, QuestionType, RetrievalConfig};

/// Path to the 7-question neutrality subset (extracted from LongMemEval_S).
/// First 5 non-temporal by question_id sort + first 2 temporal-reasoning.
const DATASET_PATH: &str = "/tmp/neutrality_5q.json";

#[test]
fn neutrality_hit_keys() {
    let data = std::fs::read_to_string(DATASET_PATH).unwrap_or_else(|e| {
        panic!("Cannot read {DATASET_PATH}: {e}. Run the extract script first.")
    });
    let questions: Vec<spectral_bench_accuracy::dataset::Question> =
        serde_json::from_str(&data).unwrap();
    assert_eq!(questions.len(), 7, "expected 7 neutrality questions");

    let dir = tempfile::tempdir().unwrap();
    let config = RetrievalConfig { max_results: 40 };

    println!("=== NEUTRALITY PROOF: hit keys per question ===");

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
                let (_formatted, hits) =
                    retrieval::retrieve_topk_fts(&brain, &q.question, &config, question_date)
                        .unwrap();
                hits.iter().map(|h| h.key.clone()).collect()
            }
            retrieval::RetrievalPath::Cascade => {
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

    println!("=== END ===");
}
