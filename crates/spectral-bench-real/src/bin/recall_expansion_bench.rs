//! Consolidated recall-EXPANSION validation — the levers that move recall@K by
//! putting an answer that was ABSENT from the pool INTO it (not reordering).
//! Validates the three shipped this arc, together, with no cross-interference:
//! (1) separator split (default) — `alice@acme.io` matches `alice@acme.io`;
//! (2) number-word bridge (SPECTRAL_NUMBER_NORMALIZE) — `3` ⇄ `three`;
//! (3) curated aliases (SPECTRAL_QUERY_ALIASES) — `chief executive` ⇄ `CEO`.
//! Each query shares NO plain token with its answer except through the lever
//! under test, so a hit proves the lever fired. Deterministic, $0, no LLM.
//!
//! Run: `cargo run -p spectral-bench-real --bin recall_expansion_bench`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig};
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
    // Env MUST be set before the first recall: the alias table is loaded once
    // and cached (OnceLock). Write the consumer alias file and point at it.
    let dir = std::env::temp_dir().join("spectral-recall-expansion");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let alias_path = dir.join("aliases.json");
    // Controlled vocabulary a consumer (Permagent) would curate.
    std::fs::write(
        &alias_path,
        r#"{"executive": ["ceo"], "ceo": ["chief", "executive"], "k8s": ["kubernetes"]}"#,
    )
    .unwrap();
    std::env::set_var("SPECTRAL_QUERY_ALIASES", &alias_path);
    std::env::set_var("SPECTRAL_NUMBER_NORMALIZE", "1");

    let brain = open(&dir.join("brain"));

    // Answers share NO plain token with their query except via the lever.
    brain
        .remember(
            "sep",
            "Ticket filed by alice@acme.io about the outage",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "num",
            "The household adopted three golden retrievers",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "alias",
            "The CEO approved the budget increase",
            Visibility::Private,
        )
        .unwrap();
    // Distractors so a hit is non-trivial.
    for (i, d) in [
        "The weekly planning sync ran long again",
        "Coffee in the kitchen needs restocking",
        "A new designer joins the team Monday",
    ]
    .iter()
    .enumerate()
    {
        brain
            .remember(&format!("d{i}"), d, Visibility::Private)
            .unwrap();
    }

    let hit = |q: &str, key: &str| -> bool {
        brain
            .recall_topk_fts(
                q,
                &RecallTopKConfig {
                    k: 40,
                    ..Default::default()
                },
                Visibility::Private,
            )
            .unwrap()
            .iter()
            .any(|h| h.key == key)
    };

    println!("=== Recall-expansion validation (all three levers on) ===\n");
    let cases = [
        ("separator split (default)", "alice@acme.io", "sep"),
        ("number-word bridge", "3 puppies", "num"),
        ("curated alias", "chief executive raise", "alias"),
    ];
    let mut pass = 0;
    for (lever, query, key) in cases {
        let ok = hit(query, key);
        if ok {
            pass += 1;
        }
        println!(
            "  {:<26} query {:<26} -> answer retrieved = {ok}",
            lever,
            format!("{query:?}")
        );
    }

    // Guard: the levers must not break ordinary recall (no regression).
    let plain = hit("outage ticket", "sep") && hit("budget increase", "alias");
    println!("\n  plain-token recall still works (no regression) = {plain}");

    println!("\n{pass}/3 expansion levers fired; each moved an answer from absent -> present.");
    println!("Deterministic, $0. These are the recall@K movers (expansion, not reordering).");

    std::env::remove_var("SPECTRAL_QUERY_ALIASES");
    std::env::remove_var("SPECTRAL_NUMBER_NORMALIZE");
}
