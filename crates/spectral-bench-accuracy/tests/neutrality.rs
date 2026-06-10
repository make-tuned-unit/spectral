//! Runtime neutrality proof: exercises retrieval on 5 real LongMemEval_S
//! questions (first 5 by question_id sort) and outputs ordered hit keys
//! for branch-vs-main comparison.
//!
//! Run with:
//!   cargo test -p spectral-bench-accuracy --test neutrality -- --nocapture
//!
//! Neutrality criterion: identical hit keys in identical order on branch
//! vs main for the same questions. Accuracy pass/fail may vary (actor
//! stochasticity) — hit-key identity is the neutrality criterion.

use spectral_bench_accuracy::ingest;
use spectral_bench_accuracy::retrieval::{self, RetrievalConfig};

/// Path to the 5-question neutrality subset (extracted from LongMemEval_S).
/// First 5 by question_id sort: mixed categories (single-session-user,
/// multi-session, knowledge-update).
const DATASET_PATH: &str = "/tmp/neutrality_5q.json";

#[test]
fn neutrality_hit_keys() {
    let data = std::fs::read_to_string(DATASET_PATH).unwrap_or_else(|e| {
        panic!("Cannot read {DATASET_PATH}: {e}. Run the extract script first.")
    });
    let questions: Vec<spectral_bench_accuracy::dataset::Question> =
        serde_json::from_str(&data).unwrap();
    assert_eq!(questions.len(), 5, "expected 5 neutrality questions");

    let dir = tempfile::tempdir().unwrap();
    let config = RetrievalConfig { max_results: 40 };

    println!("=== NEUTRALITY PROOF: hit keys per question ===");

    for q in &questions {
        let brain_dir = dir.path().join(format!("brain_{}", q.question_id));
        let brain =
            ingest::ingest_question(q, &brain_dir, ingest::IngestStrategy::PerTurn).unwrap();

        let question_date = q.question_date.as_deref();
        let (_formatted, hits) =
            retrieval::retrieve_topk_fts(&brain, &q.question, &config, question_date).unwrap();

        let keys: Vec<&str> = hits.iter().map(|h| h.key.as_str()).collect();
        println!(
            "{}|{}|hits={}|keys={}",
            q.question_id,
            q.question_type,
            keys.len(),
            serde_json::to_string(&keys).unwrap()
        );

        let _ = std::fs::remove_dir_all(&brain_dir);
    }

    println!("=== END ===");
}
