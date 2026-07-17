//! Spectrogram cross-wing resonance — the DORMANT namesake subsystem, measured.
//!
//! Every bench this arc ran with `enable_spectrogram: false`, so the seven
//! deterministic cognitive dimensions (entity_density, action_type,
//! decision_polarity, causal_depth, emotional_valence, temporal_specificity,
//! novelty) and `recall_cross_wing` (resonant recall) have never been measured.
//!
//! The claim under test — recognition-aware feedback: given a new DECISION, can
//! resonance surface the user's *other decisions across unrelated life domains*
//! ("you've decided like this before") when they share ZERO keywords, where FTS
//! structurally cannot? And does it stay action-type-precise (not dragging in
//! discoveries/problems from the same wings)? Deterministic, $0, no LLM.
//!
//! Run: `cargo run -p spectral-bench-real --bin spectrogram_resonance_bench`

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
        enable_spectrogram: true, // the whole point
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

fn remember_in_wing(brain: &Brain, key: &str, content: &str, wing: &str) {
    brain
        .remember_with(
            key,
            content,
            RememberOpts {
                visibility: Visibility::Private,
                wing: Some(wing.to_string()),
                ..Default::default()
            },
        )
        .unwrap();
}

fn main() {
    let brain = open(&std::env::temp_dir().join("spectral-resonance-bench"));

    // DECISIONS across four unrelated life domains — zero keyword overlap with
    // the seed (which is about databases). Same cognitive shape: a decision,
    // committed, with a reason.
    let decisions = [
        ("dec:health", "health", "Decided to switch to morning workouts because evening sessions kept getting skipped"),
        ("dec:home", "home", "Chose the fiberglass insulation for the attic since the crew quoted half the lead time"),
        ("dec:finance", "finance", "Picked the index fund over single stocks because steady growth beats gambling"),
        ("dec:travel", "travel", "Decided to take the train to the conference because flights kept getting cancelled"),
    ];
    // Same-wing NON-decisions (discoveries/problems) — resonance must NOT drag
    // these in despite domain overlap with the decisions above.
    let non_decisions = [
        (
            "disc:health",
            "health",
            "Noticed the knee pain flares up whenever the running shoes wear down",
        ),
        (
            "prob:home",
            "home",
            "The attic vent fan started rattling loudly during the last heat wave",
        ),
        (
            "disc:finance",
            "finance",
            "Realized the brokerage statement includes a hidden management fee",
        ),
        (
            "prob:travel",
            "travel",
            "The hotel booking site charged the card twice for the same night",
        ),
    ];
    for (k, w, c) in decisions.iter().chain(non_decisions.iter()) {
        remember_in_wing(&brain, k, c, w);
    }
    // The seed: a NEW decision in a fifth domain (work/databases).
    remember_in_wing(
        &brain,
        "dec:work",
        "Decided to migrate the billing service to Postgres because the old cluster kept falling over",
        "work",
    );

    println!("=== Spectrogram cross-wing resonance (dormant subsystem, first measurement) ===\n");

    // Arm 1: FTS recall from the seed's words — can keywords find the other decisions?
    let fts = brain
        .recall_topk_fts(
            "decided migrate billing postgres cluster",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();
    let fts_keys: Vec<&str> = fts.iter().map(|h| h.key.as_str()).collect();
    let fts_cross_domain = decisions
        .iter()
        .filter(|(k, _, _)| fts_keys.contains(k))
        .count();
    println!(
        "FTS recall (seed keywords) returned {} hits: {:?}",
        fts.len(),
        fts_keys
    );
    println!("  cross-domain decisions found by FTS: {fts_cross_domain}/4\n");

    // Arm 2: spectrogram resonance from the same seed.
    let res = brain
        .recall_cross_wing(
            "decided migrate billing postgres cluster",
            Visibility::Private,
            10,
        )
        .unwrap();
    let seed_key = res
        .seed_memory
        .as_ref()
        .map(|m| m.key.clone())
        .unwrap_or_default();
    println!("recall_cross_wing seed resolved to: {seed_key:?}");
    let mut found_decisions = 0usize;
    let mut false_positives = 0usize;
    for r in &res.resonant_memories {
        let is_decision = decisions.iter().any(|(k, _, _)| *k == r.memory.key);
        let is_non = non_decisions.iter().any(|(k, _, _)| *k == r.memory.key);
        if is_decision {
            found_decisions += 1;
        }
        if is_non {
            false_positives += 1;
        }
        println!(
            "  resonant: {:<14} score={:.2} dims={:?}",
            r.memory.key, r.resonance_score, r.matched_dimensions
        );
    }
    println!("\n  cross-domain decisions found by resonance: {found_decisions}/4");
    println!("  non-decision (discovery/problem) false positives: {false_positives}/4");

    println!("\nverdict:");
    println!("  FTS ceiling on zero-keyword-overlap decisions: {fts_cross_domain}/4 (structural)");
    println!("  resonance lift: {found_decisions}/4 with {false_positives} action-type errors");
    println!("  -> the 'you've decided like this before' recognition-aware feedback channel");
    println!(
        "     is {}",
        if found_decisions >= 2 && false_positives == 0 {
            "REAL and precise"
        } else if found_decisions >= 2 {
            "real but imprecise"
        } else {
            "NOT delivering (investigate dimensions/tolerances)"
        }
    );
    println!("\nDeterministic, $0, no LLM — 7 cognitive dimensions, no embeddings.");
}
