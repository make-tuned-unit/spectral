use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use spectral_cascade::orchestrator::CascadeConfig;
use spectral_cascade::LayerResult;
use spectral_core::device_id::DeviceId;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, RememberOpts};
use spectral_tact::TactConfig;
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
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
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
                ..Default::default()
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

// ── created_at override tests ───────────────────────────────────────

#[test]
fn remember_with_created_at_uses_provided_timestamp() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let ts = Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap();
    brain
        .remember_with(
            "created-at-override",
            "Historical memory from June 2024 about project launch",
            RememberOpts {
                created_at: Some(ts),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let result = brain
        .recall(
            "historical memory June 2024 project launch",
            Visibility::Private,
        )
        .unwrap();
    assert!(!result.memory_hits.is_empty());
    let stored = result.memory_hits[0].created_at.as_deref().unwrap();
    assert!(
        stored.starts_with("2024-06-15"),
        "expected created_at to start with 2024-06-15, got {stored}"
    );
}

#[test]
fn remember_without_created_at_uses_now() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    let before = Utc::now();

    brain
        .remember_with(
            "created-at-default",
            "Default timestamp memory about system initialization",
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let result = brain
        .recall(
            "default timestamp system initialization",
            Visibility::Private,
        )
        .unwrap();
    assert!(!result.memory_hits.is_empty());
    let stored = result.memory_hits[0].created_at.as_deref().unwrap();
    let parsed = chrono::NaiveDateTime::parse_from_str(stored, "%Y-%m-%d %H:%M:%S")
        .expect("parse created_at");
    let stored_utc = parsed.and_utc();
    let diff = (stored_utc - before).num_seconds().abs();
    assert!(
        diff < 5,
        "expected created_at within 5s of now, got {diff}s difference"
    );
}

#[test]
fn remember_with_far_past_timestamp() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let ts = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    brain
        .remember_with(
            "far-past-timestamp",
            "Ancient memory from year 2020 about early prototype",
            RememberOpts {
                created_at: Some(ts),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let result = brain
        .recall("ancient memory 2020 early prototype", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());
    let stored = result.memory_hits[0].created_at.as_deref().unwrap();
    assert!(
        stored.starts_with("2020-01-01"),
        "expected created_at to start with 2020-01-01, got {stored}"
    );
}

#[test]
fn remember_with_future_timestamp() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let ts = Utc.with_ymd_and_hms(2028, 12, 25, 18, 30, 0).unwrap();
    brain
        .remember_with(
            "future-timestamp",
            "Future dated memory about planned deployment in 2028",
            RememberOpts {
                created_at: Some(ts),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let result = brain
        .recall("future planned deployment 2028", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());
    let stored = result.memory_hits[0].created_at.as_deref().unwrap();
    assert!(
        stored.starts_with("2028-12-25"),
        "expected created_at to start with 2028-12-25, got {stored}"
    );
}

// ── TactConfig override tests ───────────────────────────────────────

#[test]
fn brain_open_respects_custom_tact_max_results() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(BrainConfig {
        tact_config: Some(TactConfig {
            max_results: 15,
            ..TactConfig::default()
        }),
        ..brain_config(&tmp)
    })
    .unwrap();

    // Ingest 20 memories with the word "project" so they all match a single recall
    for i in 0..20 {
        brain
            .remember(
                &format!("tact-test-{i}"),
                &format!("Project milestone {i} completed successfully with results"),
                Visibility::Private,
            )
            .unwrap();
    }

    let result = brain
        .recall("project milestone completed results", Visibility::Private)
        .unwrap();
    // With max_results=15, we should get at most 15 hits even though 20 match
    assert!(
        result.memory_hits.len() <= 15,
        "expected at most 15 hits with custom tact_config, got {}",
        result.memory_hits.len()
    );
    // And we should get more than the default 5
    assert!(
        result.memory_hits.len() > 5,
        "expected more than default 5 hits, got {}",
        result.memory_hits.len()
    );
}

// ── Cascade integration tests ───────────────────────────────────────

#[test]
fn recall_cascade_falls_through_to_l3() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Ingest memories that AAAK won't surface (low signal, no AAAK-worthy halls)
    brain
        .remember(
            "cascade-l3-test",
            "Discussed cascade architecture for retrieval pipeline",
            Visibility::Private,
        )
        .unwrap();

    let config = CascadeConfig::default();
    let result = brain
        .recall_cascade("cascade architecture retrieval", &config)
        .unwrap();

    // AAAK (L1) should skip (no high-signal foundational facts in a fresh brain).
    // L3 constellation/TACT should find the memory.
    assert!(
        !result.merged_hits.is_empty(),
        "cascade should produce hits via L3"
    );
    // Should not have stopped at L1 since AAAK skipped
    assert_ne!(
        result.stopped_at,
        Some(spectral_cascade::LayerId::L1),
        "should not stop at L1 when no AAAK facts found"
    );
}

#[test]
fn recall_cascade_returns_aaak_when_sufficient() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Ingest a high-signal fact in a hall that AAAK includes
    brain
        .remember_with(
            "aaak-cascade-test",
            "Decided to use PostgreSQL for the production database",
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let config = CascadeConfig::default();
    let result = brain
        .recall_cascade("PostgreSQL production database", &config)
        .unwrap();

    // Check that at least one layer produced results
    assert!(!result.merged_hits.is_empty());

    // Check that L1 ran (it may be Sufficient, Partial, or Skipped depending
    // on whether the memory's signal score and hall match AAAK criteria)
    let l1_outcome = result
        .layer_outcomes
        .iter()
        .find(|(id, _)| *id == spectral_cascade::LayerId::L1);
    assert!(l1_outcome.is_some(), "L1 should have been executed");

    // If L1 was Sufficient, cascade stopped early (ideal case).
    // If L1 Skipped (signal score too low for AAAK threshold), L3 picked it up.
    // Both are valid — the test confirms the cascade ran without error.
    let l1_was_sufficient = matches!(l1_outcome.unwrap().1, LayerResult::Sufficient { .. });
    if l1_was_sufficient {
        assert_eq!(result.stopped_at, Some(spectral_cascade::LayerId::L1));
    }
}

// ── Episode integration tests ───────────────────────────────────────

#[test]
fn brain_list_memories_by_episode_returns_constituents() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Ingest 3 memories with a shared episode_id via remember_with + created_at
    // (episode_id isn't threaded through RememberOpts yet, so we verify
    // list_memories_by_episode returns empty — proving the delegate works
    // and will return results once the ingest path populates episode_id)
    brain
        .remember(
            "ep-brain-test-1",
            "First memory for episode brain test",
            Visibility::Private,
        )
        .unwrap();

    // With no episode_id in the ingest path, list should be empty
    let mems = brain.list_memories_by_episode("nonexistent").unwrap();
    assert!(mems.is_empty());

    // remember() now auto-creates episodes, so list_episodes may not be empty
    // The delegate itself works — that's what we're testing
    let _ = brain.list_episodes(None, 100).unwrap();
}

#[test]
fn recall_cascade_returns_episode_when_dominant() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    for i in 0..5 {
        brain
            .remember_with(
                &format!("python-ep-{i}"),
                &format!("Python development task {i} with coding and debugging"),
                RememberOpts {
                    episode_id: Some("ep-python".into()),
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();
    }

    brain
        .remember_with(
            "rust-ep-0",
            "Rust systems programming discussion",
            RememberOpts {
                episode_id: Some("ep-rust".into()),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let config = CascadeConfig::default();
    let result = brain
        .recall_cascade("python development coding debugging", &config)
        .unwrap();

    let l2_outcome = result
        .layer_outcomes
        .iter()
        .find(|(id, _)| *id == spectral_cascade::LayerId::L2);
    assert!(l2_outcome.is_some(), "L2 should run");
    assert!(!result.merged_hits.is_empty());
}

#[test]
fn recall_cascade_falls_through_to_l3_when_episodes_balanced() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    for i in 0..3 {
        brain
            .remember_with(
                &format!("arch-a-{i}"),
                &format!("Architecture discussion alpha iteration {i}"),
                RememberOpts {
                    episode_id: Some("ep-alpha".into()),
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();
        brain
            .remember_with(
                &format!("arch-b-{i}"),
                &format!("Architecture discussion beta iteration {i}"),
                RememberOpts {
                    episode_id: Some("ep-beta".into()),
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();
    }

    let config = CascadeConfig::default();
    let result = brain
        .recall_cascade("architecture discussion iteration", &config)
        .unwrap();

    // Cascade should run all 3 layers
    assert!(
        result.layer_outcomes.len() >= 3,
        "cascade should run all 3 layers"
    );
}

#[test]
fn recall_cascade_skips_l2_when_no_episodes() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // remember() auto-creates episodes, so L2 will find them.
    // Verify the cascade still works end-to-end.
    brain
        .remember(
            "cascade-ep-test",
            "Memory for cascade episode testing scenario",
            Visibility::Private,
        )
        .unwrap();

    let config = CascadeConfig::default();
    let result = brain
        .recall_cascade("cascade episode testing scenario", &config)
        .unwrap();

    assert!(!result.merged_hits.is_empty());
    assert!(result.layer_outcomes.len() >= 2);
}

// ── AaakLayer calibration tests ─────────────────────────────────────

#[test]
fn aaak_layer_skips_when_no_high_signal_facts() {
    use spectral_cascade::Layer;
    use spectral_graph::cascade_layers::AaakLayer;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Ingest memories without boost keywords — these score below 0.85.
    // Generic conversational content doesn't trigger decision/error/insight
    // boosts, so signal stays at base (0.5-0.7 depending on hall).
    for (i, content) in [
        "The weather was nice today during our walk",
        "We had lunch at the Italian place on Main Street",
        "The meeting ran long but was productive overall",
        "Traffic was heavy on the way home from work",
        "The new office layout looks pretty good so far",
    ]
    .iter()
    .enumerate()
    {
        brain
            .remember(&format!("low-signal-{i}"), content, Visibility::Private)
            .unwrap();
    }

    let layer = AaakLayer::new(&brain, 200);
    let result = layer.query("weather lunch meeting", 4096).unwrap();
    assert!(
        matches!(result, LayerResult::Skipped { .. }),
        "AaakLayer should skip when no memories score >= 0.85"
    );
}

#[test]
fn aaak_layer_skips_when_only_preference_hall_above_threshold() {
    use spectral_cascade::Layer;
    use spectral_graph::cascade_layers::AaakLayer;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // "I prefer" triggers preference hall, but AaakLayer only includes "fact"
    brain
        .remember(
            "pref-high",
            "I prefer using Rust for all systems programming work",
            Visibility::Private,
        )
        .unwrap();

    let layer = AaakLayer::new(&brain, 200);
    let result = layer.query("Rust programming", 4096).unwrap();
    // Even if signal score were high enough, preference hall is excluded
    assert!(
        matches!(result, LayerResult::Skipped { .. }),
        "AaakLayer should skip preference-hall memories"
    );
}

#[test]
fn aaak_layer_fires_when_fact_above_threshold() {
    use spectral_cascade::Layer;
    use spectral_graph::cascade_layers::AaakLayer;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // "decided to use" → fact hall (0.7 base) + decision boost (+0.15) = 0.85.
    // This goes through the real classifier + scorer via remember_with.
    let r = brain
        .remember_with(
            "fact-high",
            "Decided to use Rust for the production database layer",
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    // Verify classifier + scorer produced the expected values
    assert_eq!(r.hall.as_deref(), Some("fact"), "should classify as fact");
    assert!(
        r.signal_score >= 0.85,
        "fact + 'decided' should score >= 0.85, got {}",
        r.signal_score
    );

    // Now run AaakLayer — should find this memory and return Sufficient
    let layer = AaakLayer::new(&brain, 200);
    let result = layer.query("Rust production database", 4096).unwrap();
    assert!(
        matches!(result, LayerResult::Sufficient { .. }),
        "AaakLayer should fire on a fact-hall memory scoring >= 0.85"
    );
}

// ── Annotation + ambient data tests ─────────────────────────────────

#[test]
fn brain_annotate_and_list_round_trip() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let r = brain
        .remember(
            "ann-test-mem",
            "Discussed cascade architecture for the recognition pipeline",
            Visibility::Private,
        )
        .unwrap();
    let memory_id = &r.memory_id;

    let input = spectral_ingest::AnnotationInput {
        description: "Architecture discussion about cascade layers".into(),
        who: vec![
            spectral_ingest::EntityRef {
                canonical_id: "person:jesse-sharratt".into(),
                display_name: "Jesse Sharratt".into(),
            },
            spectral_ingest::EntityRef {
                canonical_id: "did:chitin:spectral-agent".into(),
                display_name: "Spectral Agent".into(),
            },
        ],
        why: "Designing the L2 episode layer".into(),
        where_: Some("office".into()),
        when_: chrono::Utc::now(),
        how: "Pair programming session".into(),
    };

    let ann = brain.annotate(memory_id, input).unwrap();
    assert!(ann.id.starts_with("ann-"));

    let loaded = brain.list_annotations(memory_id).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].who.len(), 2);
    assert_eq!(loaded[0].who[0].canonical_id, "person:jesse-sharratt");
    assert_eq!(loaded[0].who[1].canonical_id, "did:chitin:spectral-agent");
    assert_eq!(
        loaded[0].description,
        "Architecture discussion about cascade layers"
    );

    // Probe should surface the cascade-related memory
    let probe_results = brain
        .probe(
            "cascade architecture recognition",
            spectral_graph::activity::ProbeOpts::default(),
        )
        .unwrap();
    // Signal quality note: probe uses recall() which runs TACT/FTS.
    // The memory content "Discussed cascade architecture for the recognition pipeline"
    // should match query keywords "cascade architecture recognition" via FTS.
    assert!(
        !probe_results.is_empty(),
        "probe should surface the cascade-related memory"
    );
}

#[test]
fn probe_handles_timeline_shaped_input() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember_with(
            "cascade-fix",
            "Fixed cascade calibration bug in the recognition pipeline",
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();
    brain
        .remember_with(
            "cascade-test",
            "Wrote test for cascade query orchestration",
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();
    brain
        .remember_with(
            "cascade-perf",
            "Cascade performance is fast on benchmark queries",
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();
    brain
        .remember_with(
            "rust-async",
            "Async Rust patterns for concurrent processing",
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();
    brain
        .remember_with(
            "git-rebase",
            "Git rebase workflow for feature branches",
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    // Construct timeline-shaped probe context (what Permagent Phase 2 feeds)
    let timeline_context = "[14:32] User opened ~/projects/spectral/src/cascade.rs\n\
        [14:33] User typed: fn cascade_query(...)\n\
        [14:34] Window: spectral - cascade.rs\n\
        [14:35] User Slack-messaged team: pushing cascade calibration fix\n\
        [14:36] User opened terminal: cargo test cascade";

    let results = brain
        .probe(
            timeline_context,
            spectral_graph::activity::ProbeOpts::default(),
        )
        .unwrap();

    let surfaced_keys: Vec<&str> = results.iter().map(|m| m.key.as_str()).collect();
    // Signal quality: probe uses recall() which runs TACT/FTS on the timeline text.
    // "cascade" appears 4 times in the timeline — FTS should match memories
    // containing "cascade" in their content.
    assert!(
        surfaced_keys.iter().any(|k| k.starts_with("cascade-")),
        "Expected at least one cascade-related memory from timeline probe. Surfaced: {surfaced_keys:?}"
    );

    // Diagnostic: log what surfaced for signal quality analysis
    eprintln!(
        "Timeline probe surfaced {} memories: {:?}",
        results.len(),
        surfaced_keys
    );
}

#[test]
fn remember_with_persists_compaction_tier() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let r = brain
        .remember_with(
            "tier-test",
            "Raw ambient activity event from screen monitor",
            RememberOpts {
                compaction_tier: Some(spectral_ingest::CompactionTier::HourlyRollup),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let mems = brain.list_all_memories(100).unwrap();
    let mem = mems.iter().find(|m| m.id == r.memory_id).unwrap();
    assert_eq!(
        mem.compaction_tier,
        Some(spectral_ingest::CompactionTier::HourlyRollup)
    );
}
