//! Anticipatory-recall demonstration — the read-time ambient loop.
//!
//! Ties the pieces together: usage builds co-retrieval history; the lift
//! recommender then surfaces a memory the user's current query does NOT match
//! but that their context is specifically associated with — "what you need
//! before you ask" — while suppressing a globally-popular blob (the bias that
//! sank raw co-retrieval). End-to-end through the real
//! recall -> retrieval_events -> rebuild_co_retrieval_index -> recommend loop
//! (no synthetic pairs). Deterministic, $0, no LLM.
//!
//! Run: `cargo run -p spectral-bench-real --bin anticipatory_bench`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig};
use std::path::Path;

fn open(dir: &Path) -> Brain {
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

fn recall(brain: &Brain, q: &str) -> Vec<String> {
    brain
        .recall_topk_fts(q, &RecallTopKConfig::default(), Visibility::Private)
        .unwrap()
        .into_iter()
        .map(|h| h.key)
        .collect()
}

fn main() {
    let dir = std::env::temp_dir().join("spectral-anticipatory");
    let _ = std::fs::remove_dir_all(&dir);
    let brain = open(&dir);

    // Memories. A and B are about the same incident but share few query terms;
    // "popular" is a generic status memory that matches many broad queries.
    let a = brain.remember("kube-deploy", "The production deploy runs on Kubernetes with blue-green rollouts", Visibility::Private).unwrap().memory_id;
    brain.remember("deploy-outage", "Postmortem: the outage was a bad ingress config pushed during the release window", Visibility::Private).unwrap();
    brain.remember("status", "The team met to review project status and agree on next steps this week", Visibility::Private).unwrap();
    for (k, c) in [
        ("infra-cost", "Cloud infrastructure cost review flagged the staging cluster as over-provisioned"),
        ("hiring", "Two engineering candidates cleared the final onsite and got offers"),
        ("roadmap", "The Q3 roadmap review moved the search rewrite to Q4"),
    ] {
        brain.remember(k, c, Visibility::Private).unwrap();
    }

    // ── Build usage history (retrieval_events -> co_retrieval_pairs) ──
    // The incident pair (deploy + outage) is retrieved together repeatedly:
    // whenever the user works on the deploy, they pull up the outage too.
    for _ in 0..5 {
        let _ = recall(&brain, "deploy release outage ingress"); // returns kube-deploy + deploy-outage
    }
    // The status memory is retrieved alongside MANY unrelated things (broad,
    // generic) — this is what makes it globally popular but not specific.
    for q in ["team status review", "project status week", "review next steps", "team meeting review", "status update review"] {
        for _ in 0..4 {
            let _ = recall(&brain, q);
        }
    }
    let pairs = brain.rebuild_co_retrieval_index().unwrap();
    println!("=== Anticipatory recall (read-time ambient loop) ===");
    println!("co_retrieval_pairs built from real usage: {pairs}\n");

    // ── The moment of truth ──
    // The user now asks about Kubernetes. FTS matches the kube-deploy memory
    // but NOT the outage postmortem (which shares no query terms with "kubernetes").
    let q = "kubernetes rollout strategy";
    let hits = recall(&brain, q);
    let got_outage_from_query = hits.iter().any(|k| k == "deploy-outage");
    println!("query: {q:?}");
    println!("  recall (query-match) returned: {hits:?}");
    println!("  -> outage postmortem surfaced by the query alone: {got_outage_from_query}");

    // Anticipate: given the top hit (kube-deploy), recommend associated memories
    // by LIFT. The outage postmortem — specifically associated from usage —
    // should surface; the globally-popular status memory should be suppressed.
    let recs = brain.recommend(&a, 5, 1).unwrap();
    println!("\n  recommend(top hit) by lift:");
    for r in &recs {
        println!("    {:<16} lift={:.2} co_count={}", r.memory_id, r.lift, r.co_count);
    }
    // Map ids back to keys for readability.
    let key_of = |id: &str| -> String {
        brain.get_memory(id).ok().flatten().map(|m| m.key).unwrap_or_else(|| id.to_string())
    };
    let top_rec = recs.first().map(|r| key_of(&r.memory_id));
    println!("\n  top anticipated memory: {:?}", top_rec);
    println!(
        "  => the outage postmortem the query MISSED is surfaced by anticipation: {}",
        recs.iter().any(|r| key_of(&r.memory_id) == "deploy-outage")
    );
    println!(
        "  => the globally-popular status blob is NOT the top recommendation: {}",
        top_rec.as_deref() != Some("status")
    );
    println!("\nDeterministic, $0, no LLM. recall(query) + recommend(top hit) = anticipatory");
    println!("recall: surface what the user needs, including what they didn't ask for,");
    println!("ranked by context-specific association (lift), not popularity.");

    // ── In-recall augmentation (SPECTRAL_ANTICIPATORY_RECALL) ──
    // The same anticipation, now folded INTO recall so a consumer gets
    // miss-recovery for free (no manual recommend() composition). Flag-gated,
    // default OFF; appended after the query-matches, visibility-filtered.
    println!("\n--- in-recall augmentation (SPECTRAL_ANTICIPATORY_RECALL) ---");
    let show = |label: &str| {
        let keys = recall(&brain, q);
        let outage = keys.iter().any(|k| k == "deploy-outage");
        println!("  {label:<4} recall({q:?}) => {keys:?}  outage_surfaced={outage}");
    };
    std::env::remove_var("SPECTRAL_ANTICIPATORY_RECALL");
    show("OFF");
    std::env::set_var("SPECTRAL_ANTICIPATORY_RECALL", "1");
    show("ON");
    std::env::remove_var("SPECTRAL_ANTICIPATORY_RECALL");
    println!("\n  OFF: query alone misses the postmortem. ON: recall itself surfaces it.");
    println!("  Consumers get anticipatory miss-recovery without composing recommend().");
}
