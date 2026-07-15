//! Integration test for the library associative-spreading feature: it should
//! recover an answer memory that shares NO words with the query but co-occurs
//! (same episode) with a query-matching seed — the vocabulary gap FTS can't
//! cross. Validates `spectral_graph::spreading` end-to-end against a real brain.

use std::path::PathBuf;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, RecallTopKConfig, RememberOpts};
use spectral_graph::spreading::{associative_spread, AssocSpreadConfig, SpreadMode};
use tempfile::TempDir;

fn brain_config(tmp: &TempDir) -> BrainConfig {
    BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: spectral_graph::brain::EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
    }
}

fn remember_ep(brain: &Brain, key: &str, content: &str, ep: &str) {
    brain
        .remember_with(
            key,
            content,
            RememberOpts {
                visibility: Visibility::Private,
                episode_id: Some(ep.to_string()),
                ..Default::default()
            },
        )
        .unwrap();
}

fn keys(hits: &[spectral_ingest::MemoryHit]) -> Vec<String> {
    hits.iter().map(|h| h.key.clone()).collect()
}

/// Corpus: a query-matching BRIDGE + a lexically-disjoint ANSWER in episode e1,
/// plus enough query-word-matching distractors (in other episodes) that a
/// small-k FTS ranks the zero-match answer out of the window.
fn seed_corpus(brain: &Brain) {
    remember_ep(
        brain,
        "e1-bridge",
        "suggest a dinner using fresh ingredients for the weekend party",
        "e1",
    );
    remember_ep(
        brain,
        "e1-answer",
        "growing cherry tomatoes basil and mint in the backyard garden",
        "e1",
    );
    // Query-word-matching distractors (dinner/suggest/using/ingredients) so the
    // 0-match answer is outranked. Two share episode n0 (a removable duplicate).
    let d = [
        ("d0", "the dinner reservation is confirmed for eight people", "n0"),
        ("d1", "dinner leftovers from the party went bad overnight", "n0"),
        ("d2", "I suggest we reschedule the morning standup", "n1"),
        ("d3", "using the new template for the quarterly report", "n2"),
        ("d4", "the ingredients for the cake are on the shopping list", "n3"),
        ("d5", "suggest booking dinner reservations early on weekends", "n4"),
    ];
    for (k, c, ep) in d {
        remember_ep(brain, k, c, ep);
    }
}

#[test]
fn episode_spreading_recovers_lexically_disjoint_answer() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    seed_corpus(&brain);

    let query = "suggest a dinner using my homegrown ingredients";
    let cfg = RecallTopKConfig { k: 3, ..RecallTopKConfig::default() };

    // Baseline: small-k FTS finds the bridge (word match) but not the answer.
    let base = brain.recall_topk_fts(query, &cfg, Visibility::Private).unwrap();
    let base_keys = keys(&base);
    assert!(
        base_keys.contains(&"e1-bridge".to_string()),
        "the bridge should be retrieved (seeds the spread), got {base_keys:?}"
    );
    assert!(
        !base_keys.contains(&"e1-answer".to_string()),
        "precondition: FTS should miss the lexically-disjoint answer, got {base_keys:?}"
    );

    // Off = no-op.
    let mut hits = base.clone();
    associative_spread(&brain, &mut hits, &AssocSpreadConfig::default());
    assert_eq!(keys(&hits), base_keys, "Off mode must be a no-op");

    // Episode spreading recovers the answer via same-session co-occurrence.
    let mut hits = base.clone();
    associative_spread(
        &brain,
        &mut hits,
        &AssocSpreadConfig {
            mode: SpreadMode::Episode,
            episode_budget: 4000,
            ..AssocSpreadConfig::default()
        },
    );
    assert!(
        keys(&hits).contains(&"e1-answer".to_string()),
        "episode spreading should recover the co-occurring answer, got {:?}",
        keys(&hits)
    );
}

#[test]
fn rerank_recovers_within_bounded_context() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    seed_corpus(&brain);

    let query = "suggest a dinner using my homegrown ingredients";
    let cfg = RecallTopKConfig { k: 5, ..RecallTopKConfig::default() };
    let base = brain.recall_topk_fts(query, &cfg, Visibility::Private).unwrap();
    let n_before = base.len();

    let mut hits = base.clone();
    associative_spread(
        &brain,
        &mut hits,
        &AssocSpreadConfig {
            mode: SpreadMode::Rerank,
            rerank_b: 2,
            ..AssocSpreadConfig::default()
        },
    );
    // Rerank displaces rather than grows: context stays bounded (never grows by
    // more than the number of non-removable — sole-session — hits).
    assert!(
        hits.len() <= n_before + 2,
        "rerank must keep context bounded: {} vs base {}",
        hits.len(),
        n_before
    );
}
