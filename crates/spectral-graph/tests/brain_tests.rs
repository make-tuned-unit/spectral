use std::path::PathBuf;

use spectral_core::device_id::DeviceId;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, RememberOpts};
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
        .assert("Mark", "studies", "Carol", 0.9, Visibility::Private)
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

    let result = brain.recall("Mark", Visibility::Private).unwrap();
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
    let result = brain.recall("Library", Visibility::Private).unwrap();
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

    let result = brain.recall("xyznonexistent", Visibility::Private).unwrap();
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
            "Carol works on Spectral every day",
            Visibility::Private,
        )
        .unwrap();

    assert_eq!(result.document_id.len(), 32);
    assert!(result.matched.len() >= 2); // Carol, Spectral
    let canonicals: Vec<&str> = result
        .matched
        .iter()
        .map(|m| m.canonical.as_str())
        .collect();
    assert!(canonicals.contains(&"carol-doe"));
    assert!(canonicals.contains(&"spectral"));
}

#[test]
fn ingest_document_idempotent() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let content = "Carol studies Library Science";
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
        .remember(
            "auth_decision",
            "Alice decided to use Clerk for auth",
            Visibility::Private,
        )
        .unwrap();

    // recall with enough words to pass TACT's min_words gate
    let result = brain
        .recall("what was the auth decision", Visibility::Private)
        .unwrap();
    // Should find memory via FTS fallback
    assert!(!result.memory_hits.is_empty() || !result.graph.seed_entities.is_empty());
}

#[test]
fn remember_idempotent() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let r1 = brain
        .remember("same_key", "some content here", Visibility::Private)
        .unwrap();
    let r2 = brain
        .remember("same_key", "updated content here", Visibility::Private)
        .unwrap();

    // Same key produces same memory ID (deterministic from key)
    assert_eq!(r1.memory_id, r2.memory_id);
}

#[test]
fn remember_classifies_correctly() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let result = brain
        .remember(
            "auth_decision",
            "Alice decided to use Clerk for auth",
            Visibility::Private,
        )
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

    // Remember apollo observations
    brain
        .remember(
            "apollo-decision",
            "Decided to use Apollo for the weather prediction strategy",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "apollo-bug",
            "Apollo had a bug in the weather engine",
            Visibility::Private,
        )
        .unwrap();

    // Recall with a query that matches the "apollo" wing
    let result = brain
        .recall("apollo weather strategy", Visibility::Private)
        .unwrap();
    assert!(
        !result.memory_hits.is_empty(),
        "expected memory hits for apollo wing query, got 0"
    );
}

#[test]
fn visibility_filters_memories() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Store one Private and one Public memory
    brain
        .remember(
            "private-secret",
            "Alice chose a secret auth provider",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "public-announcement",
            "Alice chose Clerk for the public API",
            Visibility::Public,
        )
        .unwrap();

    // Public context: should see only Public memory
    let public = brain
        .recall("what did Alice choose", Visibility::Public)
        .unwrap();
    assert!(
        public.memory_hits.iter().all(|m| m.visibility == "public"),
        "Public context should not see private memories"
    );

    // Private context: should see both
    let private = brain
        .recall("what did Alice choose", Visibility::Private)
        .unwrap();
    assert!(
        private.memory_hits.len() >= public.memory_hits.len(),
        "Private context should see at least as much as Public"
    );
}

#[test]
fn visibility_filters_graph_triples() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Assert one Private and one Org-visible fact
    brain
        .assert("Mark", "studies", "Library", 0.9, Visibility::Private)
        .unwrap();
    brain
        .assert("Mark", "prepares_for", "Exam", 0.9, Visibility::Org)
        .unwrap();

    // Org context: should see only the Org triple
    let org_result = brain.recall_graph("Mark", Visibility::Org).unwrap();
    assert!(
        org_result
            .triples
            .iter()
            .all(|t| t.visibility >= Visibility::Org),
        "Org context should not see Private triples"
    );

    // Private context: should see both
    let private_result = brain.recall_graph("Mark", Visibility::Private).unwrap();
    assert!(
        private_result.triples.len() > org_result.triples.len(),
        "Private should see more triples than Org"
    );
}

#[test]
fn visibility_federation_precedent() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Assert Private and Org facts
    brain
        .assert("Carol", "works_on", "Spectral", 0.9, Visibility::Private)
        .unwrap();
    brain
        .assert("Carol", "knows", "Mark", 0.9, Visibility::Org)
        .unwrap();

    // Org-context recall: Private fact must be filtered out
    let result = brain.recall("Carol", Visibility::Org).unwrap();
    for t in &result.graph.triples {
        assert!(
            t.visibility >= Visibility::Org,
            "federation leak: Private triple {:?} visible in Org context",
            t.predicate
        );
    }
}

// ── Provenance field tests ───────────────────────────────────────────

#[test]
fn remember_with_source_persists_source() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember_with(
            "apollo-native",
            "Decided to use Apollo for weather prediction",
            RememberOpts {
                source: Some("native".into()),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let result = brain
        .recall("apollo weather prediction", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());
    assert_eq!(result.memory_hits[0].source.as_deref(), Some("native"));
}

#[test]
fn remember_with_device_id_persists() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let device = DeviceId::from_descriptor("test-laptop-abc");
    brain
        .remember_with(
            "apollo-device",
            "Decided to use Apollo for weather prediction via device",
            RememberOpts {
                device_id: Some(device),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let result = brain
        .recall("apollo weather prediction device", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());
    assert_eq!(
        result.memory_hits[0].device_id.as_ref(),
        Some(device.as_bytes())
    );
}

#[test]
fn remember_with_confidence_persists() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember_with(
            "low-confidence",
            "Decided to use Apollo for weather prediction maybe",
            RememberOpts {
                confidence: Some(0.5),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let result = brain
        .recall("apollo weather prediction maybe", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());
    assert!((result.memory_hits[0].confidence - 0.5).abs() < f64::EPSILON);
}

#[test]
fn default_remember_uses_none_source_and_full_confidence() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let r = brain
        .remember(
            "default-test",
            "Decided to use Apollo for weather prediction default",
            Visibility::Private,
        )
        .unwrap();

    assert!(r.source.is_none());
    assert!((r.confidence - 1.0).abs() < f64::EPSILON);
}

#[test]
fn device_id_deterministic_from_descriptor() {
    let a = DeviceId::from_descriptor("hostname-abc");
    let b = DeviceId::from_descriptor("hostname-abc");
    assert_eq!(a, b);

    let c = DeviceId::from_descriptor("hostname-xyz");
    assert_ne!(a, c);
}

#[test]
fn schema_migration_adds_columns_idempotent() {
    use spectral_ingest::sqlite_store::SqliteStore;

    // Create a store with the old schema (no provenance columns)
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_migrate.db");

    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id            TEXT PRIMARY KEY,
                key           TEXT NOT NULL UNIQUE,
                content       TEXT NOT NULL,
                category      TEXT NOT NULL DEFAULT 'core',
                wing          TEXT DEFAULT NULL,
                hall          TEXT DEFAULT NULL,
                signal_score  REAL DEFAULT 0.5,
                visibility    TEXT NOT NULL DEFAULT 'private',
                created_at    TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                key, content, content=memories, content_rowid=rowid
            );
            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, key, content)
                VALUES (new.rowid, new.key, new.content);
            END;
            CREATE TABLE IF NOT EXISTS constellation_fingerprints (
                id TEXT PRIMARY KEY,
                fingerprint_hash TEXT NOT NULL,
                anchor_memory_id TEXT NOT NULL,
                target_memory_id TEXT NOT NULL,
                wing TEXT, anchor_hall TEXT, target_hall TEXT,
                time_delta_bucket TEXT, created_at TEXT
            );",
        )
        .unwrap();
    }

    // Open with SqliteStore — migration should add columns
    let _store = SqliteStore::open(&db_path).unwrap();

    // Open again — migration should be idempotent
    let _store2 = SqliteStore::open(&db_path).unwrap();

    // Verify columns exist by inserting with them
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "INSERT INTO memories (id, key, content, source, device_id, confidence)
         VALUES ('t1', 'k1', 'c1', 'native', X'0102030405060708091011121314151617181920212223242526272829303132', 0.75)",
        [],
    )
    .unwrap();

    let source: Option<String> = conn
        .query_row("SELECT source FROM memories WHERE id = 't1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(source.as_deref(), Some("native"));
}

#[test]
fn recall_returns_source_and_device_and_confidence() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let device = DeviceId::from_descriptor("roundtrip-host");
    brain
        .remember_with(
            "roundtrip-key",
            "Decided to use Apollo for weather prediction roundtrip",
            RememberOpts {
                source: Some("openbird_sidecar".into()),
                device_id: Some(device),
                confidence: Some(0.95),
                visibility: Visibility::Private,
            },
        )
        .unwrap();

    let result = brain
        .recall("apollo weather prediction roundtrip", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());

    let hit = &result.memory_hits[0];
    assert_eq!(hit.source.as_deref(), Some("openbird_sidecar"));
    assert_eq!(hit.device_id.as_ref(), Some(device.as_bytes()));
    assert!((hit.confidence - 0.95).abs() < f64::EPSILON);
}
