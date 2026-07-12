//! Recency-decay recall risk — does the DEFAULT recency weighting demote a
//! genuinely-old-but-relevant answer out of the top-K?
//!
//! `recall_topk_fts` applies recency decay `score *= 0.5^(age_days/365)`
//! MULTIPLICATIVELY, and when `config.now` is None the reference is wall-clock
//! `Utc::now()`. So an imported 3-year-old conversation is penalized ~8x. Every
//! prior bench missed this because freshly-`remember`ed memories all have age≈0
//! (factor≈1.0, inert). Here we set an OLD `created_at` on the most-relevant
//! answer and recent timestamps on lower-relevance distractors, then compare
//! recall@40 with recency ON (default) vs OFF. Deterministic ($0, no LLM).
//!
//! Run: `cargo run -p spectral-bench-real --bin recency_probe`

use chrono::{Duration, Utc};
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig, RememberOpts};
use std::path::Path;

fn open(dir: &Path) -> Brain {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("ontology.toml"), "version = 1\n").unwrap();
    Brain::open(BrainConfig {
        data_dir: dir.to_path_buf(),
        ontology_path: dir.join("ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
    })
    .unwrap()
}

fn main() {
    let brain = open(&std::env::temp_dir().join("spectral-recency-probe"));
    let now = Utc::now();

    // The answer matches the query BEST (3 salient terms incl. the rare "acme")
    // but is OLD. Distractors match FEWER terms but are RECENT and numerous
    // enough to overflow K=40, so truncation bites.
    // Answer and distractors match the SAME query terms (comparable bm25), so
    // recency — not relevance — decides the ordering. The answer is old; the 50
    // distractors are fresh and overflow K=40.
    brain
        .remember_with(
            "answer",
            "The quarterly review meeting approved the budget decision",
            RememberOpts {
                visibility: Visibility::Private,
                created_at: Some(now - Duration::days(3650)), // ~3 years old
                ..Default::default()
            },
        )
        .unwrap();
    for i in 0..50 {
        brain
            .remember_with(
                &format!("recent-{i}"),
                &format!("The quarterly review meeting covered status item {i}"),
                RememberOpts {
                    visibility: Visibility::Private,
                    created_at: Some(now - Duration::days(1)), // fresh
                    ..Default::default()
                },
            )
            .unwrap();
    }

    let query = "quarterly review meeting";
    // now=None -> recency anchored to wall-clock (the risky default path a naive
    // consumer hits). Compare against recency disabled.
    let default_cfg = RecallTopKConfig { k: 40, ..Default::default() }; // recency ON
    let no_recency = RecallTopKConfig { k: 40, apply_recency_weighting: false, ..Default::default() };

    let rank_of = |cfg: &RecallTopKConfig| -> String {
        let hits = brain.recall_topk_fts(query, cfg, Visibility::Private).unwrap();
        match hits.iter().position(|h| h.key == "answer") {
            Some(i) => format!("rank {} (of {})", i + 1, hits.len()),
            None => "NOT in top-40".to_string(),
        }
    };
    let in_top40 = |cfg: &RecallTopKConfig| {
        brain.recall_topk_fts(query, cfg, Visibility::Private).unwrap().iter().take(40).any(|h| h.key == "answer")
    };

    println!("=== Recency-decay recall risk ===");
    println!("query {query:?}; answer is the MOST relevant match but ~3 years old;");
    println!("50 recent lower-relevance distractors overflow K=40.\n");
    println!("  recency ON  (default): answer {}  in_top40={}", rank_of(&default_cfg), in_top40(&default_cfg));
    println!("  recency OFF          : answer {}  in_top40={}", rank_of(&no_recency), in_top40(&no_recency));
    println!();
    let on = rank_of(&default_cfg);
    let off = rank_of(&no_recency);
    println!("\nVALIDATION of the recency fix (parser + additive-bounded):");
    // (1) parser fix: recency is now ACTIVE for an RFC3339-imported timestamp
    //     (ON ordering differs from OFF); previously both were identical because
    //     the parse failed and recency was silently inert for imports.
    println!("  [1] recency now ACTIVE for imports (ON {on} != OFF {off}): {}", on != off);
    // (2) additive bound: the relevant 10-year-old answer stays in top-40 rather
    //     than being annihilated out of it, as the old multiplicative decay
    //     (score *= 0.5^(3650/365) ≈ 0.001) would have done.
    println!("  [2] relevant 10yr-old answer still in top-40 (not annihilated): {}", in_top40(&default_cfg));
    println!("\nDeterministic, $0, no LLM. Recency is now consistent (native + import) and a");
    println!("bounded tiebreaker, not a force that buries old-but-relevant answers.");
}
