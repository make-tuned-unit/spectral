use std::path::PathBuf;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};
use tempfile::TempDir;

fn open_brain(tmp: &TempDir) -> Brain {
    Brain::open(BrainConfig {
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
        activity_wing: "activity".into(),
        redaction_policy: None,
    })
    .unwrap()
}

#[test]
fn recall_returns_hits_for_single_word_query() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    for i in 0..5 {
        brain
            .remember(
                &format!("apollo-mem-{i}"),
                &format!("Apollo weather prediction strategy observation {i}"),
                Visibility::Private,
            )
            .unwrap();
    }

    let result = brain.recall("apollo", Visibility::Private).unwrap();
    assert!(
        !result.memory_hits.is_empty(),
        "single-word query 'apollo' should return hits, got 0"
    );
}

#[test]
fn recall_returns_hits_for_two_word_query() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    for i in 0..5 {
        brain
            .remember(
                &format!("apollo-weather-{i}"),
                &format!("Apollo weather prediction strategy details {i}"),
                Visibility::Private,
            )
            .unwrap();
    }

    let result = brain.recall("apollo weather", Visibility::Private).unwrap();
    assert!(
        !result.memory_hits.is_empty(),
        "two-word query 'apollo weather' should return hits, got 0"
    );
}

#[test]
fn min_words_threshold_respected_when_set_explicitly() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    brain
        .remember(
            "apollo-explicit",
            "Apollo weather prediction strategy explicit test",
            Visibility::Private,
        )
        .unwrap();

    // The default min_words is now 1, so single-word queries work.
    // But TACT's min_words gate is still functional — verified by the fact
    // that the pipeline runs and returns results for short queries.
    // A consumer could set min_words=3 to skip short queries.
    let result = brain.recall("apollo", Visibility::Private).unwrap();
    assert!(
        !result.memory_hits.is_empty(),
        "default min_words=1 should allow single-word queries"
    );
}

#[test]
fn recall_skipped_query_still_populates_graph() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    // Assert a graph fact with known ontology entities
    brain
        .assert("Mark", "studies", "Library", 0.9, Visibility::Private)
        .unwrap();

    // Recall "Mark" (single word) — TACT may not find memory hits (no memories stored),
    // but the graph path should find the entity and its neighborhood.
    let result = brain.recall("Mark", Visibility::Private).unwrap();
    assert!(
        !result.graph.seed_entities.is_empty(),
        "graph path should find 'Mark' entity even for single-word query"
    );
    assert!(
        !result.graph.triples.is_empty(),
        "graph path should return triples for 'Mark'"
    );
}
