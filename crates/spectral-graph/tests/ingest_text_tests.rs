use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, IngestTextOpts};
use spectral_tact::LlmClient;
use tempfile::TempDir;

struct MockLlmClient {
    canned_response: String,
}

impl LlmClient for MockLlmClient {
    fn complete(
        &self,
        _prompt: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + '_>> {
        let response = self.canned_response.clone();
        Box::pin(async move { Ok(response) })
    }
}

struct FailingLlmClient;

impl LlmClient for FailingLlmClient {
    fn complete(
        &self,
        _prompt: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + '_>> {
        Box::pin(async move { Err(anyhow::anyhow!("LLM unavailable")) })
    }
}

fn brain_with_llm(tmp: &TempDir, client: Box<dyn LlmClient>) -> Brain {
    Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        memory_db_path: None,
        llm_client: Some(client),
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: spectral_graph::brain::EntityPolicy::Strict,
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
    })
    .unwrap()
}

fn brain_without_llm(tmp: &TempDir) -> Brain {
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
        tact_config: None,
    })
    .unwrap()
}

#[test]
fn ingest_text_extracts_simple_triple() {
    let tmp = TempDir::new().unwrap();
    let client = MockLlmClient {
        canned_response: r#"{"triples": [{"subject": "Mark", "predicate": "studies", "object": "Library", "confidence": 0.9}]}"#.into(),
    };
    let brain = brain_with_llm(&tmp, Box::new(client));

    let result = brain
        .ingest_text(
            "Mark studies Library Science",
            IngestTextOpts {
                visibility: Visibility::Private,
                min_confidence: 0.5,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(result.triples_extracted, 1);
    assert_eq!(result.triples_asserted, 1);
    assert!(result.triples_rejected.is_empty());

    // Verify the triple was actually persisted in the graph
    let recall = brain.recall_graph("Mark", Visibility::Private).unwrap();
    assert!(!recall.triples.is_empty());
    assert_eq!(recall.triples[0].predicate, "studies");
}

#[test]
fn ingest_text_rejects_below_confidence() {
    let tmp = TempDir::new().unwrap();
    let client = MockLlmClient {
        canned_response: r#"{"triples": [{"subject": "Mark", "predicate": "studies", "object": "Library", "confidence": 0.3}]}"#.into(),
    };
    let brain = brain_with_llm(&tmp, Box::new(client));

    let result = brain
        .ingest_text(
            "Mark studies Library Science",
            IngestTextOpts {
                visibility: Visibility::Private,
                min_confidence: 0.5,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(result.triples_extracted, 1);
    assert_eq!(result.triples_asserted, 0);
    assert_eq!(result.triples_rejected.len(), 1);

    match &result.triples_rejected[0].reason {
        spectral_graph::brain::RejectionReason::BelowConfidenceThreshold => {}
        other => panic!("expected BelowConfidenceThreshold, got {:?}", other),
    }
}

#[test]
fn ingest_text_rejects_invalid_predicate() {
    let tmp = TempDir::new().unwrap();
    let client = MockLlmClient {
        canned_response: r#"{"triples": [{"subject": "Mark", "predicate": "invented_pred", "object": "Library", "confidence": 0.9}]}"#.into(),
    };
    let brain = brain_with_llm(&tmp, Box::new(client));

    let result = brain
        .ingest_text(
            "Mark studies Library Science",
            IngestTextOpts {
                visibility: Visibility::Private,
                min_confidence: 0.5,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(result.triples_extracted, 1);
    assert_eq!(result.triples_asserted, 0);
    assert_eq!(result.triples_rejected.len(), 1);

    match &result.triples_rejected[0].reason {
        spectral_graph::brain::RejectionReason::InvalidPredicate(p) => {
            assert_eq!(p, "invented_pred");
        }
        other => panic!("expected InvalidPredicate, got {:?}", other),
    }
}

#[test]
fn ingest_text_stores_memory_too() {
    let tmp = TempDir::new().unwrap();
    let client = MockLlmClient {
        canned_response: r#"{"triples": [{"subject": "Mark", "predicate": "studies", "object": "Library", "confidence": 0.9}]}"#.into(),
    };
    let brain = brain_with_llm(&tmp, Box::new(client));

    let text = "Mark studies Library Science at the university";
    let result = brain
        .ingest_text(
            text,
            IngestTextOpts {
                visibility: Visibility::Private,
                min_confidence: 0.5,
                ..Default::default()
            },
        )
        .unwrap();

    // The memory should have been stored
    assert!(!result.memory.memory_id.is_empty());

    // Should be recoverable via recall
    let recall = brain
        .recall("Mark studies Library", Visibility::Private)
        .unwrap();
    assert!(
        !recall.memory_hits.is_empty() || !recall.graph.seed_entities.is_empty(),
        "ingested text should be findable via recall"
    );
}

#[test]
fn ingest_text_without_llm_client_errors() {
    let tmp = TempDir::new().unwrap();
    let brain = brain_without_llm(&tmp);

    let err = brain
        .ingest_text("Mark studies Library Science", IngestTextOpts::default())
        .unwrap_err();

    match err {
        spectral_graph::Error::MissingLlmClient => {}
        other => panic!("expected MissingLlmClient, got {:?}", other),
    }
}

#[test]
fn ingest_text_handles_malformed_llm_response() {
    let tmp = TempDir::new().unwrap();
    let client = MockLlmClient {
        canned_response: "I'm sorry, I can't extract any triples from that.".into(),
    };
    let brain = brain_with_llm(&tmp, Box::new(client));

    let result = brain
        .ingest_text(
            "Mark studies Library Science",
            IngestTextOpts {
                visibility: Visibility::Private,
                min_confidence: 0.5,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(result.triples_extracted, 0);
    assert_eq!(result.triples_asserted, 0);
    assert!(result.triples_rejected.is_empty());
    // Memory should still be stored even with zero triples
    assert!(!result.memory.memory_id.is_empty());
}

#[test]
fn ingest_text_full_pipeline() {
    let tmp = TempDir::new().unwrap();
    // Return 3 triples:
    // 1. valid: Mark studies Library (confidence 0.9)
    // 2. invalid predicate: Mark invented_pred Library (confidence 0.8)
    // 3. below confidence: Mark prepares_for Exam (confidence 0.3)
    let client = MockLlmClient {
        canned_response: r#"{"triples": [
            {"subject": "Mark", "predicate": "studies", "object": "Library", "confidence": 0.9},
            {"subject": "Mark", "predicate": "invented_pred", "object": "Library", "confidence": 0.8},
            {"subject": "Mark", "predicate": "prepares_for", "object": "Exam", "confidence": 0.3}
        ]}"#
        .into(),
    };
    let brain = brain_with_llm(&tmp, Box::new(client));

    let result = brain
        .ingest_text(
            "Mark studies Library Science and prepares for the Final Exam",
            IngestTextOpts {
                visibility: Visibility::Private,
                min_confidence: 0.5,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(result.triples_extracted, 3);
    assert_eq!(result.triples_asserted, 1); // only "studies" passes
    assert_eq!(result.triples_rejected.len(), 2);

    // Verify rejection reasons
    let reasons: Vec<&str> = result
        .triples_rejected
        .iter()
        .map(|r| match &r.reason {
            spectral_graph::brain::RejectionReason::BelowConfidenceThreshold => "confidence",
            spectral_graph::brain::RejectionReason::InvalidPredicate(_) => "predicate",
            spectral_graph::brain::RejectionReason::UnresolvedSubject => "subject",
            spectral_graph::brain::RejectionReason::UnresolvedObject => "object",
        })
        .collect();
    assert!(reasons.contains(&"predicate"));
    assert!(reasons.contains(&"confidence"));

    // Verify the graph has exactly the one valid triple
    let recall = brain.recall_graph("Mark", Visibility::Private).unwrap();
    assert_eq!(recall.triples.len(), 1);
    assert_eq!(recall.triples[0].predicate, "studies");
}

#[test]
fn ingest_text_rejects_unresolved_subject() {
    let tmp = TempDir::new().unwrap();
    let client = MockLlmClient {
        canned_response: r#"{"triples": [{"subject": "UnknownPerson", "predicate": "studies", "object": "Library", "confidence": 0.9}]}"#.into(),
    };
    let brain = brain_with_llm(&tmp, Box::new(client));

    let result = brain
        .ingest_text(
            "UnknownPerson studies Library Science",
            IngestTextOpts {
                visibility: Visibility::Private,
                min_confidence: 0.5,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(result.triples_asserted, 0);
    assert_eq!(result.triples_rejected.len(), 1);
    match &result.triples_rejected[0].reason {
        spectral_graph::brain::RejectionReason::UnresolvedSubject => {}
        other => panic!("expected UnresolvedSubject, got {:?}", other),
    }
}

#[test]
fn ingest_text_llm_error_propagates() {
    let tmp = TempDir::new().unwrap();
    let brain = brain_with_llm(&tmp, Box::new(FailingLlmClient));

    let err = brain
        .ingest_text("Mark studies Library Science", IngestTextOpts::default())
        .unwrap_err();

    match err {
        spectral_graph::Error::Llm(msg) => {
            assert!(msg.contains("unavailable"));
        }
        other => panic!("expected Llm error, got {:?}", other),
    }
}

#[test]
fn ingest_text_custom_memory_key() {
    let tmp = TempDir::new().unwrap();
    let client = MockLlmClient {
        canned_response: r#"{"triples": []}"#.into(),
    };
    let brain = brain_with_llm(&tmp, Box::new(client));

    let result = brain
        .ingest_text(
            "Some text here",
            IngestTextOpts {
                memory_key: Some("my-custom-key".into()),
                ..Default::default()
            },
        )
        .unwrap();

    // The memory key is hashed into the ID, so just verify it went through
    assert!(!result.memory.memory_id.is_empty());
}
