//! AAAK — the "always-in-prompt" curated memory layer, measured for the first time.
//!
//! `Brain::aaak` builds a ~170-token summary of the highest-signal durable facts
//! (halls: fact/preference/decision/rule, signal ≥ 0.7), greedy by signal_score.
//! It's Spectral's L3-persona / TencentDB-L3 analogue: a tiny block injected into
//! every system prompt so the agent ALWAYS knows the user's standing facts —
//! even for a query whose keywords wouldn't retrieve them.
//!
//! Measured here: (1) does it respect the token budget; (2) does it prioritize
//! durable preferences/rules/decisions over ephemeral chatter; (3) the payoff —
//! a standing constraint ("vegetarian", "allergic to shellfish") that a
//! keyword recall for an unrelated request ("suggest a restaurant") would MISS,
//! but AAAK always carries. Deterministic, $0, no LLM.
//!
//! Run: `cargo run -p spectral-bench-real --bin aaak_curated_bench`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{AaakOpts, Brain, BrainConfig, EntityPolicy};
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
    let brain = open(&std::env::temp_dir().join("spectral-aaak"));

    // Durable standing facts — the things an agent should ALWAYS know.
    let durable = [
        (
            "pref:diet",
            "I am vegetarian and do not eat any meat or fish",
        ),
        (
            "pref:allergy",
            "I am severely allergic to shellfish and peanuts",
        ),
        (
            "rule:nocall",
            "Never schedule anything for me before 9am, I do not take morning calls",
        ),
        (
            "decision:stack",
            "I decided to standardize all my projects on Rust and Postgres",
        ),
        (
            "fact:family",
            "My daughter Mia is five years old and starts school in September",
        ),
        (
            "pref:comm",
            "I prefer concise written summaries over long meetings",
        ),
    ];
    for (k, c) in durable {
        brain.remember(k, c, Visibility::Private).unwrap();
    }
    // Ephemeral chatter — high volume, low durable value.
    for i in 0..30 {
        brain
            .remember(
                &format!("chat{i}"),
                &format!("Grabbed a coffee and answered some emails around item {i} today"),
                Visibility::Private,
            )
            .unwrap();
    }

    println!("=== AAAK curated always-in-prompt layer (dormant, first measurement) ===\n");

    // Diagnose the upstream gate: AAAK filters by hall ∈ {fact,preference,
    // decision,rule} AND signal ≥ 0.7. Show what the classifier actually assigned.
    println!("classifier assignment for the durable facts (AAAK needs hall∈set AND signal≥0.7):");
    let id_of = |k: &str| -> String {
        format!(
            "{:016x}",
            u64::from_be_bytes(
                blake3::hash(k.as_bytes()).as_bytes()[..8]
                    .try_into()
                    .unwrap()
            )
        )
    };
    for (k, _) in durable {
        if let Ok(Some(m)) = brain.get_memory(&id_of(k)) {
            let pass = m
                .hall
                .as_deref()
                .map(|h| ["fact", "preference", "decision", "rule"].contains(&h))
                .unwrap_or(false)
                && m.signal_score >= 0.7;
            println!(
                "  {k:<16} hall={:<12} signal={:.2}  {}",
                format!("{:?}", m.hall),
                m.signal_score,
                if pass { "✓ eligible" } else { "✗ excluded" }
            );
        }
    }
    println!();

    let res = brain.aaak(AaakOpts::default()).unwrap();
    println!(
        "budget: 170 tokens | AAAK used {} tokens, {} facts, wings: {:?}",
        res.estimated_tokens, res.fact_count, res.wings_represented
    );
    println!("within budget: {}\n", res.estimated_tokens <= 170);
    println!("AAAK block:\n{}", res.formatted);

    // Which durable facts made the cut? Which chatter leaked in?
    let durable_in = durable
        .iter()
        .filter(|(_, c)| res.formatted.contains(&c[..c.len().min(30)]))
        .count();
    let chatter_in = res.formatted.matches("Grabbed a coffee").count();
    println!("durable facts included: {durable_in}/{}", durable.len());
    println!("ephemeral chatter leaked in: {chatter_in}\n");

    // The payoff: a request whose keywords would NOT recall the dietary/allergy
    // constraints, but which the agent must respect — AAAK carries them.
    let q = "suggest a restaurant for dinner tonight";
    let recalled = brain
        .recall_topk_fts(q, &Default::default(), Visibility::Private)
        .unwrap();
    let recall_has_diet = recalled
        .iter()
        .any(|h| h.key == "pref:diet" || h.key == "pref:allergy");
    let aaak_has_diet = res.formatted.contains("vegetarian") || res.formatted.contains("allergic");
    println!("query: {q:?}");
    println!("  keyword recall surfaces the dietary/allergy constraint: {recall_has_diet}");
    println!("  AAAK (always present) carries it:                       {aaak_has_diet}");

    println!("\nverdict:");
    let ok = res.estimated_tokens <= 170
        && durable_in >= 4
        && chatter_in == 0
        && aaak_has_diet
        && !recall_has_diet;
    println!(
        "  AAAK is {}",
        if ok {
            "delivering: within budget, durable facts prioritized, chatter excluded, and it\n  carries a standing constraint that query-keyword recall structurally misses"
        } else {
            "partial — inspect selection above"
        }
    );
    println!("\nDeterministic, $0, no LLM. ~170 tokens injected into every prompt.");
}
