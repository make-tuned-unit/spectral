use std::path::PathBuf;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};
use tempfile::TempDir;

fn brain_config(tmp: &TempDir) -> BrainConfig {
    BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
    }
}

#[test]
fn open_creates_data_dir_contents() {
    let tmp = TempDir::new().unwrap();
    let _brain = Brain::open(brain_config(&tmp)).unwrap();

    assert!(tmp.path().join("brain.key").exists());
    assert!(tmp.path().join("brain.pub").exists());
    assert!(tmp.path().join("brain.id").exists());
    assert!(tmp.path().join("graph.kz").exists());
}

#[test]
fn open_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let brain1 = Brain::open(brain_config(&tmp)).unwrap();
    let id1 = brain1.brain_id().to_string();
    drop(brain1);

    let brain2 = Brain::open(brain_config(&tmp)).unwrap();
    let id2 = brain2.brain_id().to_string();

    assert_eq!(id1, id2, "second open should find existing identity");
}

#[test]
fn assert_valid_fact() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let result = brain
        .assert("Mark", "studies", "Library", 0.9, Visibility::Private)
        .unwrap();

    assert!(result.triple_written);
    assert_eq!(result.subject.canonical, "mark-smith");
    assert_eq!(result.predicate, "studies");
    assert_eq!(result.object.canonical, "library-science");
}

#[test]
fn assert_unresolved_subject() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let err = brain
        .assert("Nobody", "studies", "Library", 0.9, Visibility::Private)
        .unwrap_err();

    match err {
        spectral_graph::Error::UnresolvedMention { mention, .. } => {
            assert_eq!(mention, "Nobody");
        }
        other => panic!("expected UnresolvedMention, got {:?}", other),
    }
}

#[test]
fn assert_invalid_predicate() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // "studies" has domain=person, range=topic. Using person→person should fail.
    let err = brain
        .assert("Mark", "studies", "Sophie", 0.9, Visibility::Private)
        .unwrap_err();

    match err {
        spectral_graph::Error::InvalidPredicate {
            predicate,
            subject_type,
            object_type,
        } => {
            assert_eq!(predicate, "studies");
            assert_eq!(subject_type, "person");
            assert_eq!(object_type, "person");
        }
        other => panic!("expected InvalidPredicate, got {:?}", other),
    }
}

#[test]
fn recall_returns_asserted_facts() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .assert("Mark", "studies", "Library", 0.9, Visibility::Private)
        .unwrap();

    let result = brain.recall("Mark").unwrap();
    assert!(!result.graph.seed_entities.is_empty());
    assert!(!result.graph.triples.is_empty());
    assert_eq!(result.graph.triples[0].predicate, "studies");
}

#[test]
fn recall_multi_hop_cognee_example() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Mark studies Library Science
    brain
        .assert("Mark", "studies", "Library", 0.9, Visibility::Private)
        .unwrap();
    // Mark prepares for Final Exam
    brain
        .assert("Mark", "prepares_for", "Exam", 0.9, Visibility::Private)
        .unwrap();

    // Recall "Library" should find Library → Mark → Exam (2-hop)
    let result = brain.recall("Library").unwrap();
    assert!(result.graph.neighborhood.entities.len() >= 3);
    assert!(result.graph.neighborhood.triples.len() >= 2);

    // Verify we can find both the "studies" and "prepares_for" triples
    let predicates: Vec<&str> = result
        .graph
        .neighborhood
        .triples
        .iter()
        .map(|t| t.predicate.as_str())
        .collect();
    assert!(predicates.contains(&"studies"));
    assert!(predicates.contains(&"prepares_for"));
}

#[test]
fn recall_unknown_query_empty() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let result = brain.recall("xyznonexistent").unwrap();
    assert!(result.graph.seed_entities.is_empty());
    assert!(result.graph.triples.is_empty());
}

#[test]
fn ingest_document_writes_mentions() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let result = brain
        .ingest_document(
            "test.txt",
            "Sophie works on Spectral every day",
            Visibility::Private,
        )
        .unwrap();

    assert_eq!(result.document_id.len(), 32);
    assert!(result.matched.len() >= 2); // Sophie, Spectral
    let canonicals: Vec<&str> = result
        .matched
        .iter()
        .map(|m| m.canonical.as_str())
        .collect();
    assert!(canonicals.contains(&"sophie-sharratt"));
    assert!(canonicals.contains(&"spectral"));
}

#[test]
fn ingest_document_idempotent() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let content = "Sophie studies Library Science";
    let r1 = brain
        .ingest_document("doc.txt", content, Visibility::Private)
        .unwrap();
    let r2 = brain
        .ingest_document("doc.txt", content, Visibility::Private)
        .unwrap();

    // Same content produces same document_id
    assert_eq!(r1.document_id, r2.document_id);
}

#[test]
fn remember_and_recall_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember("auth_decision", "Jesse decided to use Clerk for auth")
        .unwrap();

    // recall with enough words to pass TACT's min_words gate
    let result = brain.recall("what was the auth decision").unwrap();
    // Should find memory via FTS fallback
    assert!(!result.memory_hits.is_empty() || !result.graph.seed_entities.is_empty());
}

#[test]
fn remember_idempotent() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let r1 = brain.remember("same_key", "some content here").unwrap();
    let r2 = brain.remember("same_key", "updated content here").unwrap();

    // Same key produces same memory ID (deterministic from key)
    assert_eq!(r1.memory_id, r2.memory_id);
}

#[test]
fn remember_classifies_correctly() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let result = brain
        .remember("auth_decision", "Jesse decided to use Clerk for auth")
        .unwrap();

    assert_eq!(result.hall.as_deref(), Some("fact"));
    assert!(result.signal_score >= 0.7);
}

#[test]
fn open_creates_memory_db() {
    let tmp = TempDir::new().unwrap();
    let _brain = Brain::open(brain_config(&tmp)).unwrap();

    assert!(tmp.path().join("memory.db").exists());
}

#[test]
fn recall_returns_memory_hits_for_matching_wing() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Remember polybot observations
    brain
        .remember(
            "polybot-decision",
            "Decided to use Polybot for the weather prediction strategy",
        )
        .unwrap();
    brain
        .remember("polybot-bug", "Polybot had a bug in the weather engine")
        .unwrap();

    // Recall with a query that matches the "polybot" wing
    let result = brain.recall("polybot weather strategy").unwrap();
    assert!(
        !result.memory_hits.is_empty(),
        "expected memory hits for polybot wing query, got 0"
    );
}
