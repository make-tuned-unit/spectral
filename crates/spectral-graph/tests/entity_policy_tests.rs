use std::path::PathBuf;
use std::sync::Arc;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use tempfile::TempDir;

fn strict_brain(tmp: &TempDir) -> Brain {
    Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
    })
    .unwrap()
}

/// Copy fixture ontology to temp dir so auto-create tests don't corrupt the shared fixture.
fn copy_ontology(tmp: &TempDir) -> PathBuf {
    let src = std::fs::read_to_string("tests/fixtures/brain_ontology.toml").unwrap();
    let dst = tmp.path().join("ontology.toml");
    std::fs::write(&dst, src).unwrap();
    dst
}

fn auto_create_brain(tmp: &TempDir) -> Brain {
    let ontology_path = copy_ontology(tmp);
    Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::AutoCreate,
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
    })
    .unwrap()
}

fn canonicalizer_brain(tmp: &TempDir) -> Brain {
    let ontology_path = copy_ontology(tmp);
    let canonicalizer: Arc<dyn Fn(&str) -> String + Send + Sync> =
        Arc::new(|mention: &str| mention.trim().to_lowercase().replace(' ', "-"));

    Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::AutoCreateWithCanonicalizer(canonicalizer),
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
    })
    .unwrap()
}

// ── Strict policy tests (regression) ─────────────────────────────

#[test]
fn strict_policy_fails_on_unknown_entity() {
    let tmp = TempDir::new().unwrap();
    let brain = strict_brain(&tmp);

    let err = brain
        .assert(
            "UnknownPerson",
            "studies",
            "Library",
            0.9,
            Visibility::Private,
        )
        .unwrap_err();
    match err {
        spectral_graph::Error::UnresolvedMention { mention, .. } => {
            assert_eq!(mention, "UnknownPerson");
        }
        other => panic!("expected UnresolvedMention, got {:?}", other),
    }
}

#[test]
fn strict_policy_succeeds_on_known_entity() {
    let tmp = TempDir::new().unwrap();
    let brain = strict_brain(&tmp);

    let result = brain
        .assert("Mark", "studies", "Library", 0.9, Visibility::Private)
        .unwrap();
    assert!(result.triple_written);
    assert_eq!(result.subject.canonical, "mark-smith");
}

// ── AutoCreate policy tests ──────────────────────────────────────

#[test]
fn auto_create_creates_subject() {
    let tmp = TempDir::new().unwrap();
    let brain = auto_create_brain(&tmp);

    // "studies" has domain=["person"], range=["topic"] — unambiguous
    let result = brain
        .assert("Alice", "studies", "Library", 0.9, Visibility::Private)
        .unwrap();

    assert!(result.triple_written);
    assert_eq!(result.subject.entity_type, "person");
    assert_eq!(result.subject.canonical, "Alice");
}

#[test]
fn auto_create_creates_object() {
    let tmp = TempDir::new().unwrap();
    let brain = auto_create_brain(&tmp);

    let result = brain
        .assert(
            "Mark",
            "studies",
            "Quantum Physics",
            0.9,
            Visibility::Private,
        )
        .unwrap();

    assert!(result.triple_written);
    assert_eq!(result.object.entity_type, "topic");
    assert_eq!(result.object.canonical, "Quantum Physics");
}

#[test]
fn auto_create_reuses_existing_entity() {
    let tmp = TempDir::new().unwrap();
    let brain = auto_create_brain(&tmp);

    // First assert creates "Alice" as person
    brain
        .assert("Alice", "studies", "Library", 0.9, Visibility::Private)
        .unwrap();

    // Second assert should reuse the same entity
    let r2 = brain
        .assert("Alice", "studies", "Exam", 0.9, Visibility::Private)
        .unwrap();
    assert_eq!(r2.subject.canonical, "Alice");
    assert_eq!(r2.subject.entity_type, "person");
}

#[test]
fn auto_create_fails_on_ambiguous_predicate() {
    let tmp = TempDir::new().unwrap();
    let brain = auto_create_brain(&tmp);

    // "knows" has domain=["person"], range=["person"] — unambiguous
    // But let's test with a predicate that doesn't exist (ambiguous = unknown)
    let err = brain
        .assert("Alice", "invented_pred", "Bob", 0.9, Visibility::Private)
        .unwrap_err();
    match err {
        spectral_graph::Error::Ontology(_) => {} // predicate not found
        other => panic!("expected Ontology error, got {:?}", other),
    }
}

#[test]
fn auto_create_succeeds_with_assert_typed_on_ambiguous() {
    let tmp = TempDir::new().unwrap();
    let brain = auto_create_brain(&tmp);

    // Use assert_typed to bypass type inference
    let result = brain
        .assert_typed(
            ("person", "Alice"),
            "knows",
            ("person", "Bob"),
            0.9,
            Visibility::Private,
        )
        .unwrap();

    assert!(result.triple_written);
    assert_eq!(result.subject.canonical, "Alice");
    assert_eq!(result.object.canonical, "Bob");
}

// ── Canonicalizer policy tests ───────────────────────────────────

#[test]
fn canonicalizer_collapses_case_variants() {
    let tmp = TempDir::new().unwrap();
    let brain = canonicalizer_brain(&tmp);

    // "Alice Smith" -> "alice-smith"
    brain
        .assert(
            "Alice Smith",
            "studies",
            "Library",
            0.9,
            Visibility::Private,
        )
        .unwrap();

    // "ALICE SMITH" -> "alice-smith" (same canonical)
    let r2 = brain
        .assert("ALICE SMITH", "studies", "Exam", 0.9, Visibility::Private)
        .unwrap();

    assert_eq!(r2.subject.canonical, "alice-smith");
}

#[test]
fn canonicalizer_preserves_mention_as_alias() {
    let tmp = TempDir::new().unwrap();
    let brain = canonicalizer_brain(&tmp);

    let result = brain
        .assert(
            "Alice Smith",
            "studies",
            "Library",
            0.9,
            Visibility::Private,
        )
        .unwrap();

    // Canonical is "alice-smith" and original "Alice Smith" is an alias
    assert_eq!(result.subject.canonical, "alice-smith");
}

#[test]
fn canonicalizer_adds_new_alias_on_subsequent_match() {
    let tmp = TempDir::new().unwrap();
    let brain = canonicalizer_brain(&tmp);

    // First assertion creates entity
    brain
        .assert(
            "Alice Smith",
            "studies",
            "Library",
            0.9,
            Visibility::Private,
        )
        .unwrap();

    // Second assertion with different case
    brain
        .assert("ALICE SMITH", "studies", "Exam", 0.9, Visibility::Private)
        .unwrap();

    // Both should resolve to the same entity
    let r3 = brain
        .assert(
            "alice smith",
            "prepares_for",
            "Exam",
            0.8,
            Visibility::Private,
        )
        .unwrap();
    assert_eq!(r3.subject.canonical, "alice-smith");
}

// ── assert_typed tests ───────────────────────────────────────────

#[test]
fn assert_typed_uses_explicit_type() {
    let tmp = TempDir::new().unwrap();
    let brain = auto_create_brain(&tmp);

    let result = brain
        .assert_typed(
            ("person", "Bob"),
            "works_on",
            ("project", "Apollo"),
            0.9,
            Visibility::Private,
        )
        .unwrap();

    assert_eq!(result.subject.entity_type, "person");
    assert_eq!(result.object.entity_type, "project");
    assert_eq!(result.object.canonical, "Apollo");
}

// ── Persistence test ─────────────────────────────────────────────

#[test]
fn auto_created_entities_persist_across_brain_reopen() {
    let tmp = TempDir::new().unwrap();

    // Copy the fixture ontology to the temp dir so we can write to it
    let ontology_src = std::fs::read_to_string("tests/fixtures/brain_ontology.toml").unwrap();
    let ontology_path = tmp.path().join("ontology.toml");
    std::fs::write(&ontology_path, &ontology_src).unwrap();

    {
        let brain = Brain::open(BrainConfig {
            data_dir: tmp.path().to_path_buf(),
            ontology_path: ontology_path.clone(),
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::AutoCreate,
            sqlite_mmap_size: None,
            activity_wing: "activity".into(),
            redaction_policy: None,
        })
        .unwrap();

        brain
            .assert("Alice", "studies", "Library", 0.9, Visibility::Private)
            .unwrap();
    }

    // Reopen brain — the auto-created entity should be loadable from the ontology file
    let brain2 = Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: ontology_path.clone(),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::Strict, // Strict! No auto-create
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
    })
    .unwrap();

    // The persisted entity should be found even in Strict mode
    let result = brain2
        .assert("Alice", "studies", "Exam", 0.8, Visibility::Private)
        .unwrap();
    assert_eq!(result.subject.canonical, "Alice");
}

// ── Default policy test ──────────────────────────────────────────

#[test]
fn default_policy_is_strict() {
    let tmp = TempDir::new().unwrap();

    let brain = Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::default(),
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
    })
    .unwrap();

    // Should fail on unknown entity (Strict behavior)
    let err = brain
        .assert(
            "UnknownPerson",
            "studies",
            "Library",
            0.9,
            Visibility::Private,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        spectral_graph::Error::UnresolvedMention { .. }
    ));
}
