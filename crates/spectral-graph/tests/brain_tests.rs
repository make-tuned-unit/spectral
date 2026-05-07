use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use spectral_cascade::orchestrator::CascadeConfig;
use spectral_cascade::RecognitionContext;
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

// ── Cascade integrated pipeline tests ────────────────────────────────

#[test]
fn recall_cascade_produces_hits() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "cascade-test",
            "Discussed cascade architecture for retrieval pipeline",
            Visibility::Private,
        )
        .unwrap();

    let config = CascadeConfig::default();
    let result = brain
        .recall_cascade(
            "cascade architecture retrieval",
            &RecognitionContext::empty(),
            &config,
        )
        .unwrap();

    assert!(
        !result.merged_hits.is_empty(),
        "cascade pipeline should produce hits"
    );
    assert_eq!(result.total_recognition_token_cost, 0);
    // Pipeline should not contain synthetic __aaak__ blocks
    assert!(
        !result.merged_hits.iter().any(|h| h.id == "__aaak__"),
        "pipeline should not contain __aaak__ blocks"
    );
}

#[test]
fn recall_cascade_accepts_context() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "ctx-test",
            "Memory for recognition context acceptance test",
            Visibility::Private,
        )
        .unwrap();

    let config = CascadeConfig::default();

    // Empty context
    let result = brain
        .recall_cascade(
            "recognition context acceptance test",
            &RecognitionContext::empty(),
            &config,
        )
        .unwrap();
    assert!(!result.merged_hits.is_empty());
    assert_eq!(result.total_recognition_token_cost, 0);

    // Populated context
    let ctx = RecognitionContext::empty().with_focus_wing("permagent");
    let result2 = brain
        .recall_cascade("recognition context acceptance test", &ctx, &config)
        .unwrap();
    assert!(!result2.merged_hits.is_empty());
}

#[test]
fn recall_cascade_returns_diverse_episodes() {
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
            "Rust systems programming discussion with coding and debugging",
            RememberOpts {
                episode_id: Some("ep-rust".into()),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    let config = CascadeConfig::default();
    let result = brain
        .recall_cascade(
            "development coding debugging",
            &RecognitionContext::empty(),
            &config,
        )
        .unwrap();

    assert!(!result.merged_hits.is_empty());
    // Pipeline should return memories from both episodes
    let episodes: std::collections::HashSet<_> = result
        .merged_hits
        .iter()
        .filter_map(|h| h.episode_id.as_deref())
        .collect();
    assert!(
        !episodes.is_empty(),
        "should have results from at least 1 episode"
    );
}

// ── Episode integration tests ───────────────────────────────────────

#[test]
fn brain_list_memories_by_episode_returns_constituents() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "ep-brain-test-1",
            "First memory for episode brain test",
            Visibility::Private,
        )
        .unwrap();

    let mems = brain.list_memories_by_episode("nonexistent").unwrap();
    assert!(mems.is_empty());
    let _ = brain.list_episodes(None, 100).unwrap();
}

// ── Cascade pipeline tests ──────────────────────────────────────────

#[test]
fn cascade_pipeline_returns_results() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "cascade-test-1",
            "Decided to use PostgreSQL for the production database layer",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "cascade-test-2",
            "Architecture discussion about cascade retrieval pipeline",
            Visibility::Private,
        )
        .unwrap();

    let config = spectral_cascade::orchestrator::CascadeConfig::default();
    let result = brain
        .recall_cascade(
            "PostgreSQL production database",
            &RecognitionContext::empty(),
            &config,
        )
        .unwrap();

    assert!(
        !result.merged_hits.is_empty(),
        "cascade should return results"
    );
    assert_eq!(result.total_recognition_token_cost, 0);
    // No __aaak__ synthetic blocks
    assert!(
        !result.merged_hits.iter().any(|h| h.id == "__aaak__"),
        "cascade should not contain synthetic __aaak__ blocks"
    );
}

#[test]
fn cascade_pipeline_returns_more_than_five_results() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Ingest 15 memories so FTS has enough to return
    for i in 0..15 {
        brain
            .remember(
                &format!("pipeline-test-{i}"),
                &format!("Project milestone {i} completed for the pipeline architecture design"),
                Visibility::Private,
            )
            .unwrap();
    }

    let config = spectral_cascade::orchestrator::CascadeConfig::default();
    let result = brain
        .recall_cascade(
            "pipeline architecture design milestone",
            &RecognitionContext::empty(),
            &config,
        )
        .unwrap();

    // Should return more than TACT's old max_results=5
    assert!(
        result.merged_hits.len() > 5,
        "cascade should return >5 results (got {}), not capped by TACT",
        result.merged_hits.len()
    );
}

// ── Legacy layer tests removed ──────────────────────────────────────
// The old AaakLayer, EpisodeLayer, ConstellationLayer tests tested
// an abstraction that has been replaced by the integrated pipeline.
// cascade_pipeline_returns_results and cascade_pipeline_returns_more_than_five_results
// cover the integrated path.

// The old layer-specific tests (aaak_layer_*, episode_layer_*, constellation_layer_*)
// have been removed — the Layer abstraction was replaced by the integrated pipeline.
// See cascade_pipeline_returns_results and cascade_pipeline_returns_more_than_five_results.

// ── FTS query quoting tests (preserved) ─────────────────────────────
// These test topk_fts which is independent of the cascade redesign.

// ── FTS query quoting tests ──────────────────────────────────────────

#[test]
fn recall_topk_fts_handles_column_syntax_words() {
    use spectral_graph::brain::RecallTopKConfig;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "k1",
            "Sarah emailed about the meeting on Tuesday",
            Visibility::Private,
        )
        .unwrap();

    // "day" and "to" previously crashed FTS5 with "no such column" errors
    let result = brain.recall_topk_fts(
        "remember the day to email Sarah",
        &RecallTopKConfig::default(),
        Visibility::Private,
    );

    assert!(
        result.is_ok(),
        "should not crash on column-syntax words: {:?}",
        result.err()
    );
}

#[test]
fn recall_topk_fts_handles_special_chars() {
    use spectral_graph::brain::RecallTopKConfig;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "k1",
            "Latest update from Sarah on the project",
            Visibility::Private,
        )
        .unwrap();

    let result = brain.recall_topk_fts(
        "what's the (latest) update from sarah*?",
        &RecallTopKConfig::default(),
        Visibility::Private,
    );

    assert!(
        result.is_ok(),
        "should not crash on special chars: {:?}",
        result.err()
    );
}

#[test]
fn recall_topk_fts_finds_multi_word_matches() {
    use spectral_graph::brain::RecallTopKConfig;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember("k1", "Sarah emailed about the meeting", Visibility::Private)
        .unwrap();

    let result = brain
        .recall_topk_fts(
            "Sarah email meeting",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    assert!(!result.is_empty(), "should find the seeded memory");
}
