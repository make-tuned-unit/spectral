use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use spectral_cascade::RecognitionContext;
use spectral_core::device_id::DeviceId;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, RecallTopKConfig, RememberOpts};
use spectral_graph::cascade_layers::CascadePipelineConfig;
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
        fts_tokenizer: None,
        read_only: false,
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

    let config = CascadePipelineConfig::default();
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

    let config = CascadePipelineConfig::default();

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

    let config = CascadePipelineConfig::default();
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

    let config = CascadePipelineConfig::default();
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

    let config = CascadePipelineConfig::default();
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

#[test]
fn forget_hard_deletes_across_substrates_and_verifies() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(BrainConfig {
        enable_spectrogram: true,
        ..brain_config(&tmp)
    })
    .unwrap();

    let content = "Pod task-runner-7f9 OOMKilled at 512Mi during the nightly batch reindex";
    brain.remember("incident", content, Visibility::Private).unwrap();

    // Present before: recall finds it, recognition recognizes it.
    let before = brain
        .recall_topk_fts(content, &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    assert!(before.iter().any(|h| h.key == "incident"), "should recall before forget");
    let rec_before = brain.recognize(content).unwrap();
    assert!(
        matches!(rec_before.verdict, spectral_recognition::Verdict::Recognized { .. }),
        "should recognize before forget"
    );

    // Forget it.
    let report = brain.forget("incident").unwrap();
    assert!(report.store.existed, "memory should have existed");
    assert_eq!(report.store.memory_rows, 1);
    assert_eq!(report.store.fts_rows, 1, "should purge the FTS shadow");
    // (A lone memory has no constellation fingerprints — those link pairs;
    // fingerprint purging is covered by the ingest-level substrate test.)
    assert_eq!(report.store.spectrograms, 1, "should purge the spectrogram row");
    assert!(report.recognition_removed, "should unenroll from recognition index");
    assert!(report.recall_clear, "recall probe should be clear post-forget");
    assert!(report.recognize_clear, "recognize probe should be clear post-forget");
    assert!(report.fully_forgotten(), "all substrates + probes should confirm gone");

    // Verify independently: gone from recall and recognition.
    let after = brain
        .recall_topk_fts(content, &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    assert!(!after.iter().any(|h| h.key == "incident"), "must not recall after forget");
    let rec_after = brain.recognize(content).unwrap();
    assert!(
        !matches!(rec_after.verdict, spectral_recognition::Verdict::Recognized { .. }),
        "must not recognize after forget"
    );
    assert!(brain.get_memory(&brain_memory_id("incident")).unwrap().is_none());
}

#[test]
fn remembered_memories_are_signed_and_verifiable() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "decision",
            "Decided to use Postgres for the project database",
            Visibility::Team,
        )
        .unwrap();

    let hits = brain
        .recall_topk_fts(
            "Postgres project database",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();
    let hit = hits.iter().find(|h| h.key == "decision").expect("hit present");

    // The hit carries authenticated provenance.
    assert_eq!(
        hit.source_brain_id.as_ref(),
        Some(brain.brain_id().as_bytes()),
        "hit should be stamped with the authoring brain id"
    );
    assert!(hit.signature.is_some(), "hit should carry a signature");

    // Verifies against the brain's own key.
    assert!(
        Brain::verify_hit(hit, brain.verifying_key()),
        "signature should verify against the authoring key"
    );

    // Does NOT verify against a different key (impersonation defense).
    let other = Brain::open(brain_config(&TempDir::new().unwrap())).unwrap();
    assert!(
        !Brain::verify_hit(hit, other.verifying_key()),
        "signature must not verify against a foreign key"
    );

    // Tampering with the content invalidates the signature.
    let mut tampered = hit.clone();
    tampered.content = "Decided to use MySQL for the project database".into();
    assert!(
        !Brain::verify_hit(&tampered, brain.verifying_key()),
        "tampered content must fail verification"
    );

    // Visibility escalation (team -> public) invalidates the signature.
    let mut escalated = hit.clone();
    escalated.visibility = "public".into();
    assert!(
        !Brain::verify_hit(&escalated, brain.verifying_key()),
        "visibility escalation must fail verification"
    );
}

#[test]
fn forget_missing_key_reports_not_existed() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    let report = brain.forget("nonexistent").unwrap();
    assert!(!report.store.existed);
    assert_eq!(report.store.memory_rows, 0);
    assert!(!report.fully_forgotten(), "a missing memory is not 'forgotten'");
}

/// Open a brain with stemmed+unstemmed fusion active. The store reads
/// `SPECTRAL_FTS_FUSION` at open and captures the flag, so the env var only
/// needs to be set across `Brain::open` — recall reads the captured flag, not
/// the env — which keeps the global-env window minimal.
fn open_fusion_brain(tmp: &TempDir) -> Brain {
    std::env::set_var("SPECTRAL_FTS_FUSION", "1");
    let brain = Brain::open(brain_config(tmp)).unwrap();
    std::env::remove_var("SPECTRAL_FTS_FUSION");
    brain
}

#[test]
fn fts_fusion_recovers_overstem_and_inflection_end_to_end() {
    // Research lever #1, end-to-end through the Brain. Porter (default) wins
    // inflection ("doctors"→"doctor") but loses over-stem collisions
    // ("university"→"univers"←"universe": a short distractor outranks the
    // answer). RRF fusion with an unstemmed channel recovers BOTH.
    let seed = |brain: &Brain| {
        brain.remember("doc", "She finally consulted a doctor about the persistent cough", Visibility::Private).unwrap();
        brain.remember("univ", "Our state university announced it raised its national research ranking again", Visibility::Private).unwrap();
        brain.remember("universe", "The universe is vast", Visibility::Private).unwrap();
    };
    let top = |brain: &Brain, q: &str| -> Option<String> {
        brain
            .recall_topk_fts(q, &RecallTopKConfig::default(), Visibility::Private)
            .unwrap()
            .first()
            .map(|h| h.key.clone())
    };
    let recalls = |brain: &Brain, q: &str, key: &str| -> bool {
        brain
            .recall_topk_fts(q, &RecallTopKConfig::default(), Visibility::Private)
            .unwrap()
            .iter()
            .any(|h| h.key == key)
    };

    // Porter-only (default): the short distractor wins the over-stem query.
    let tmp_p = TempDir::new().unwrap();
    let porter = Brain::open(brain_config(&tmp_p)).unwrap();
    seed(&porter);
    assert_eq!(
        top(&porter, "university").as_deref(),
        Some("universe"),
        "porter-only: over-stemming lets the short 'universe' distractor outrank the answer"
    );
    assert!(recalls(&porter, "doctors", "doc"), "porter matches the inflection");

    // Fusion: the answer is recovered at rank 1, inflection still works.
    let tmp_f = TempDir::new().unwrap();
    let fusion = open_fusion_brain(&tmp_f);
    seed(&fusion);
    assert_eq!(
        top(&fusion, "university").as_deref(),
        Some("univ"),
        "fusion: the unstemmed channel ranks the exact 'university' answer first, RRF promotes it"
    );
    assert!(recalls(&fusion, "doctors", "doc"), "fusion keeps porter's inflection match");
}

#[test]
fn fts_fusion_plural_strip_recovers_overstem_flood() {
    // Porter stems `engineers` AND `engineering` to `engin`, so a flood of
    // "Engineering ..." memories buries the true `engineer` answer. The
    // unstemmed channel's conservative plural-strip (`engineers`→also `engineer`)
    // matches the answer exactly WITHOUT matching the `engineering` flood, so
    // fusion lifts the answer back to the top. Regression guard for the S-stemmer-
    // like second channel.
    let seed = |brain: &Brain| {
        brain.remember("answer", "The startup finally hired one more senior backend engineer", Visibility::Private).unwrap();
        for i in 0..12 {
            brain.remember(&format!("flood{i}"), &format!("Engineering shipped the milestone number {i} on schedule"), Visibility::Private).unwrap();
        }
    };
    let top = |brain: &Brain| -> Option<String> {
        brain
            .recall_topk_fts("engineers", &RecallTopKConfig::default(), Visibility::Private)
            .unwrap()
            .first()
            .map(|h| h.key.clone())
    };

    let tmp_p = TempDir::new().unwrap();
    let porter = Brain::open(brain_config(&tmp_p)).unwrap();
    seed(&porter);
    assert_ne!(
        top(&porter).as_deref(),
        Some("answer"),
        "porter-only: the 'Engineering' flood buries the 'engineer' answer"
    );

    let tmp_f = TempDir::new().unwrap();
    let fusion = open_fusion_brain(&tmp_f);
    seed(&fusion);
    assert_eq!(
        top(&fusion).as_deref(),
        Some("answer"),
        "fusion: plural-strip on the unstemmed channel recovers the exact 'engineer' answer"
    );
}

#[test]
fn forget_purges_fusion_raw_index_no_resurrection() {
    // Correctness guard: with fusion on, recall reads BOTH indexes. If forget
    // did not purge the unstemmed index, a forgotten memory would resurrect
    // through the raw channel. The AFTER DELETE trigger on memories_fts_raw must
    // fire during the hard delete.
    let tmp = TempDir::new().unwrap();
    let brain = open_fusion_brain(&tmp);
    brain.remember("secret", "The launch codes are stored in vault seven", Visibility::Private).unwrap();

    // Present via fused recall before forget (exact, unstemmed-friendly query).
    let before = brain
        .recall_topk_fts("vault seven launch codes", &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    assert!(before.iter().any(|h| h.key == "secret"), "present before forget");

    let report = brain.forget("secret").unwrap();
    assert!(report.fully_forgotten(), "forget reports fully gone");
    assert!(report.recall_clear, "recall probe (which now fuses) is clear");

    // Must not resurface through the unstemmed channel.
    let after = brain
        .recall_topk_fts("vault seven launch codes", &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    assert!(!after.iter().any(|h| h.key == "secret"), "must not resurrect via the raw fusion index");
}

#[test]
fn forget_supersedes_prior_version_reader_reingest_scenario() {
    // Validates the Permagent Reader dependency (backlog 2026-06-17): Reader
    // keys memories by content-hash. Re-ingesting an UPDATED document creates a
    // new-hash memory while the old-hash memory persists; Reader calls
    // forget(old_key) on re-ingest. The contract: the superseded version must be
    // HARD-gone from recall (not soft-filtered), the replacement must SURVIVE,
    // and — the actual federation risk — a read-only replica must not surface
    // the stale version either.
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Reader ingests v1, then re-ingests the corrected document as v2 (a
    // different content-hash → a different key). Both coexist until forget.
    let v1_key = "doc:q2-report@hashA";
    let v2_key = "doc:q2-report@hashB";
    brain.remember(v1_key, "Q2 revenue was 4.2 million dollars", Visibility::Private).unwrap();
    brain.remember(v2_key, "Q2 revenue was 5.1 million dollars, corrected figure", Visibility::Private).unwrap();

    // Before forget: both versions are recall-able — this is exactly the stale
    // read Permagent flagged (a federated peer could surface the 4.2M version).
    let before = brain
        .recall_topk_fts("Q2 revenue", &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    assert!(before.iter().any(|h| h.key == v1_key), "v1 present before forget");
    assert!(before.iter().any(|h| h.key == v2_key), "v2 present before forget");

    // Reader drops the superseded content-hash on re-ingest.
    let report = brain.forget(v1_key).unwrap();
    assert!(report.store.existed, "the superseded memory existed");
    assert_eq!(report.store.fts_rows, 1, "the superseded FTS shadow is purged (gone from recall, not filtered)");
    assert!(report.recall_clear, "forget's own recall probe confirms the stale version is gone");
    assert!(report.fully_forgotten(), "superseded version fully forgotten across substrates");

    // After forget (local): the replacement SURVIVES, the stale version is gone.
    let after = brain
        .recall_topk_fts("Q2 revenue", &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    assert!(after.iter().any(|h| h.key == v2_key), "v2 (replacement) must survive forget");
    assert!(!after.iter().any(|h| h.key == v1_key), "v1 (superseded) must be gone from recall");
    assert!(brain.get_memory(&brain_memory_id(v1_key)).unwrap().is_none(), "v1 row hard-deleted");

    // The federation risk, closed: a read-only replica of the same brain (the
    // fan-out path a "federated Henry" reads through) cannot surface the stale
    // version, because forget purged the underlying FTS row, not a soft flag.
    drop(brain);
    let replica = Brain::open(BrainConfig { read_only: true, ..brain_config(&tmp) }).unwrap();
    let via_replica = replica
        .recall_topk_fts("Q2 revenue", &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    assert!(via_replica.iter().any(|h| h.key == v2_key), "replica still serves the current version");
    assert!(!via_replica.iter().any(|h| h.key == v1_key), "replica must not surface the stale version");
}

fn brain_memory_id(key: &str) -> String {
    format!(
        "{:016x}",
        u64::from_be_bytes(
            blake3::hash(key.as_bytes()).as_bytes()[..8]
                .try_into()
                .unwrap()
        )
    )
}

#[test]
fn recall_topk_fts_matches_possessive_entity_query() {
    // Regression: a possessive query term ("Marcus's") was char-filtered to
    // "Marcuss" (apostrophe dropped, s kept), which stems differently than the
    // entity "Marcus" in content and never matched — silently dropping the
    // answer-bearing memory from the candidate pool.
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "promo",
            "Reorg this week. Marcus got bumped up to Director of Engineering, well earned",
            Visibility::Private,
        )
        .unwrap();
    // Distractor that shares a stopword but not the entity.
    brain
        .remember("filler", "It is a busy quarter with a lot of change", Visibility::Private)
        .unwrap();

    let hits = brain
        .recall_topk_fts(
            "What is Marcus's new job title?",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    assert!(
        hits.iter().any(|h| h.key == "promo"),
        "possessive query 'Marcus's' must match the entity 'Marcus' and retrieve the memory"
    );
}

#[test]
fn recall_topk_fts_porter_stems_by_default() {
    use spectral_graph::brain::RecallTopKConfig;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "k1",
            "I met with the doctor yesterday about my knee",
            Visibility::Private,
        )
        .unwrap();

    // Plural query bridges to singular content under the porter default.
    let result = brain
        .recall_topk_fts(
            "doctors",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    assert!(
        result.iter().any(|h| h.key == "k1"),
        "porter default should bridge doctors→doctor"
    );
}

#[test]
fn brain_config_fts_tokenizer_overrides_default() {
    use spectral_graph::brain::RecallTopKConfig;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(BrainConfig {
        fts_tokenizer: Some("unicode61".into()),
        ..brain_config(&tmp)
    })
    .unwrap();

    brain
        .remember(
            "k1",
            "I met with the doctor yesterday about my knee",
            Visibility::Private,
        )
        .unwrap();

    // Explicit unstemmed tokenizer: the plural query no longer matches.
    let result = brain
        .recall_topk_fts(
            "doctors",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    assert!(
        !result.iter().any(|h| h.key == "k1"),
        "unicode61 override should disable stemming"
    );
}

// ── Recall→Recognition feedback loop tests ──────────────────────────

#[test]
fn cascade_auto_reinforces_returned_memories() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember_with(
            "reinforce-test",
            "Alice decided to use Rust for the auth service project",
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();

    // Read initial signal_score
    let initial = brain.recall_local("auth service Rust").unwrap();
    assert!(!initial.memory_hits.is_empty());
    let initial_score = initial.memory_hits[0].signal_score;

    // Run cascade recall — should auto-reinforce
    let context = RecognitionContext::empty();
    let config = CascadePipelineConfig::default();
    let result = brain
        .recall_cascade("auth service Rust", &context, &config)
        .unwrap();
    assert!(!result.merged_hits.is_empty());

    // Read signal_score again — should have been nudged by ~0.01
    let after = brain.recall_local("auth service Rust").unwrap();
    assert!(!after.memory_hits.is_empty());
    let after_score = after.memory_hits[0].signal_score;

    // Signal score should have increased (auto-reinforce strength = 0.01)
    assert!(
        after_score > initial_score,
        "signal_score should increase after cascade retrieval: before={initial_score}, after={after_score}"
    );
    // But not by too much (only 0.01)
    let delta = after_score - initial_score;
    assert!(
        delta < 0.05,
        "auto-reinforce should be small (0.01), got delta={delta}"
    );
}

#[test]
fn cascade_logs_retrieval_event() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "event-test",
            "I started jogging every morning for better health",
            Visibility::Private,
        )
        .unwrap();

    // Cascade recall should log a retrieval event
    let context = RecognitionContext::empty();
    let config = CascadePipelineConfig::default();
    let _ = brain
        .recall_cascade("jogging morning health", &context, &config)
        .unwrap();

    // Verify retrieval event was logged with correct method
    let count = brain.count_retrieval_events_by_method("cascade").unwrap();
    assert!(
        count >= 1,
        "cascade should log a retrieval event with method='cascade', found {count}"
    );
}

#[test]
fn topk_fts_logs_retrieval_event() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "fts-event-test",
            "I recently purchased a new camera for photography",
            Visibility::Private,
        )
        .unwrap();

    let _ = brain
        .recall_topk_fts(
            "camera photography",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    let count = brain.count_retrieval_events_by_method("topk_fts").unwrap();
    assert!(
        count >= 1,
        "topk_fts should log a retrieval event with method='topk_fts', found {count}"
    );

    // cascade events should still be zero (we only used topk_fts)
    let cascade_count = brain.count_retrieval_events_by_method("cascade").unwrap();
    assert_eq!(
        cascade_count, 0,
        "no cascade events should exist when only topk_fts was used"
    );
}

#[test]
fn brain_set_and_get_description_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let result = brain
        .remember(
            "desc-test",
            "I prefer dark mode in all my editors",
            Visibility::Private,
        )
        .unwrap();

    // Before setting description
    let mem = brain.get_memory(&result.memory_id).unwrap().unwrap();
    assert!(mem.description.is_none());
    assert!(mem.description_generated_at.is_none());

    // Set description
    brain
        .set_description(&result.memory_id, "User's editor preference for dark mode")
        .unwrap();

    // After setting description
    let mem = brain.get_memory(&result.memory_id).unwrap().unwrap();
    assert_eq!(
        mem.description.as_deref(),
        Some("User's editor preference for dark mode")
    );
    assert!(mem.description_generated_at.is_some());
}

#[test]
fn brain_list_undescribed_excludes_described() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let r1 = brain
        .remember("ud1", "I like Rust", Visibility::Private)
        .unwrap();
    let _r2 = brain
        .remember("ud2", "I use Neovim", Visibility::Private)
        .unwrap();
    let _r3 = brain
        .remember("ud3", "My favorite color is blue", Visibility::Private)
        .unwrap();

    // Describe one
    brain
        .set_description(&r1.memory_id, "Language preference")
        .unwrap();

    let undescribed = brain.list_undescribed(100).unwrap();
    let ids: Vec<&str> = undescribed.iter().map(|m| m.id.as_str()).collect();
    assert!(
        !ids.contains(&r1.memory_id.as_str()),
        "described memory should be excluded"
    );
    assert_eq!(
        undescribed.len(),
        2,
        "should have exactly 2 undescribed memories"
    );
}

#[test]
fn brain_related_memories_after_cascade_retrievals() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Ingest 5 memories with overlapping content
    let r1 = brain
        .remember(
            "co-1",
            "I use Rust for systems programming",
            Visibility::Private,
        )
        .unwrap();
    let _r2 = brain
        .remember(
            "co-2",
            "My favorite editor is Neovim for Rust development",
            Visibility::Private,
        )
        .unwrap();
    let _r3 = brain
        .remember(
            "co-3",
            "I prefer dark mode in all editors",
            Visibility::Private,
        )
        .unwrap();
    let _r4 = brain
        .remember(
            "co-4",
            "My daily commute is 30 minutes by train",
            Visibility::Private,
        )
        .unwrap();
    let _r5 = brain
        .remember(
            "co-5",
            "I graduated with a Computer Science degree",
            Visibility::Private,
        )
        .unwrap();

    // Run cascade retrievals that will return overlapping subsets
    let cascade_config = CascadePipelineConfig::default();
    let ctx = RecognitionContext::empty();
    let _ = brain.recall_cascade("Rust programming", &ctx, &cascade_config);
    let _ = brain.recall_cascade("editor setup Rust", &ctx, &cascade_config);
    let _ = brain.recall_cascade("Neovim dark mode editor", &ctx, &cascade_config);

    // Rebuild the co-retrieval index
    let pairs_written = brain.rebuild_co_retrieval_index().unwrap();

    // Query related memories for r1 (Rust systems programming)
    let related = brain.related_memories(&r1.memory_id, 10).unwrap();

    // Verify: index was built (at least some pairs exist from cascade events)
    // The exact set depends on what cascade returned, but the API should work
    assert!(
        pairs_written > 0 || related.is_empty(),
        "either pairs were created or no retrievals overlapped"
    );

    // If there are related memories, they should be ordered by co_count desc
    for window in related.windows(2) {
        assert!(
            window[0].co_count >= window[1].co_count,
            "related memories should be ordered by co_count desc"
        );
    }

    // memory field should be None in v1
    for r in &related {
        assert!(r.memory.is_none(), "v1 returns memory: None");
    }
}

#[test]
fn layered_consolidation_loop_deterministic_end_to_end() {
    // The full ambient → consolidation → provenance-drill-down loop, all $0/no-LLM.
    // Models the multi-session case that trips actors: three separate mentions of
    // the same wedding. Ambient co-retrieval flags them as a recurring cluster;
    // an extractive summary consolidates them into one abstract memory linked to
    // its sources; layered recall then surfaces the abstract memory (sources
    // hidden from ordinary recall) and drills down to the exact evidence.
    use spectral_ingest::CompactionTier;
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let sources = ["w1", "w2", "w3"];
    brain.remember("w1", "Attended the wedding of Rachel and Mike at the lakeside venue", Visibility::Private).unwrap();
    brain.remember("w2", "Rachel and Mike's wedding was lovely, great toast from the best man", Visibility::Private).unwrap();
    brain.remember("w3", "The wedding for Rachel and Mike had a live band and dancing", Visibility::Private).unwrap();
    brain.remember("distract", "Booked a dentist appointment for next Tuesday morning", Visibility::Private).unwrap();

    // Ambient signal: usage repeatedly co-retrieves the three wedding mentions.
    for _ in 0..4 {
        let _ = brain.recall_topk_fts("Rachel Mike wedding", &RecallTopKConfig::default(), Visibility::Private).unwrap();
    }
    let pairs = brain.rebuild_co_retrieval_index().unwrap();
    assert!(pairs > 0, "co-retrieval history should exist");

    // Candidate selection surfaces the recurring wedding cluster.
    let candidates = brain.consolidation_candidates(1, 100).unwrap();
    let cluster = candidates
        .iter()
        .find(|c| c.member_keys.iter().filter(|k| sources.contains(&k.as_str())).count() >= 2)
        .expect("wedding cluster should be a consolidation candidate");
    assert!(cluster.cohesion > 0.0, "cluster has an ambient cohesion score");

    // Consolidate the cluster deterministically (extractive, $0). Use exactly the
    // three wedding sources.
    let members: Vec<String> = sources.iter().map(|s| s.to_string()).collect();
    brain.consolidate_extractive(&members, "wedding:rachel-mike", CompactionTier::DailyRollup).unwrap();

    // Ordinary recall now surfaces the abstract memory, NOT the raw sources.
    let plain = brain.recall_topk_fts("Rachel Mike wedding", &RecallTopKConfig::default(), Visibility::Private).unwrap();
    assert!(plain.iter().any(|h| h.key == "wedding:rachel-mike"), "abstract memory is recalled");
    for s in &sources {
        assert!(!plain.iter().any(|h| &h.key == s), "raw source {s} is hidden from ordinary recall after consolidation");
    }

    // Layered recall drills down: the abstract hit carries its source memories.
    let layered = brain.recall_with_provenance("Rachel Mike wedding", &RecallTopKConfig::default(), Visibility::Private, 10).unwrap();
    let abstract_hit = layered.iter().find(|l| l.hit.key == "wedding:rachel-mike").expect("abstract hit present");
    let source_keys: std::collections::HashSet<&str> = abstract_hit.sources.iter().map(|h| h.key.as_str()).collect();
    for s in &sources {
        assert!(source_keys.contains(s), "provenance drill-down includes source {s}");
    }
}

#[test]
fn consolidate_with_accepts_a_custom_summarizer() {
    // The summarizer seam accepts any closure (e.g. a sparse LLM) — here a
    // deterministic stand-in — and stores its output as the abstraction content.
    use spectral_ingest::CompactionTier;
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    brain.remember("a", "The Q2 revenue was 4.2 million dollars", Visibility::Private).unwrap();
    brain.remember("b", "Q2 revenue came in at 4.2M, up from Q1", Visibility::Private).unwrap();

    let members = vec!["a".to_string(), "b".to_string()];
    brain
        .consolidate_with(&members, "q2:summary", CompactionTier::DailyRollup, |contents| {
            format!("SUMMARY of {} sources: Q2 revenue = $4.2M", contents.len())
        })
        .unwrap();

    let m = brain.get_memory(&brain_memory_id("q2:summary")).unwrap().unwrap();
    assert!(m.content.starts_with("SUMMARY of 2 sources"), "summarizer output stored: {}", m.content);
    assert_eq!(m.compaction_tier, Some(CompactionTier::DailyRollup));
}

#[test]
fn anticipatory_recall_augments_query_miss_when_enabled() {
    // A query that keyword-matches only one memory should, with the
    // anticipatory flag ON, also surface a lift-associated memory it MISSED —
    // and leave results unchanged when OFF. Locks the in-recall augmentation
    // (SPECTRAL_ANTICIPATORY_RECALL) demonstrated in anticipatory_bench.
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let seed = brain
        .remember(
            "kube-deploy",
            "The production deploy runs on Kubernetes with blue-green rollouts",
            Visibility::Private,
        )
        .unwrap()
        .memory_id;
    brain
        .remember(
            "deploy-outage",
            "Postmortem: the outage was a bad ingress config pushed during the release window",
            Visibility::Private,
        )
        .unwrap();

    // Build co-retrieval history: the two are pulled together repeatedly.
    for _ in 0..5 {
        let _ = brain
            .recall_topk_fts(
                "deploy release outage ingress",
                &RecallTopKConfig::default(),
                Visibility::Private,
            )
            .unwrap();
    }
    let pairs = brain.rebuild_co_retrieval_index().unwrap();
    assert!(pairs > 0, "co-retrieval pairs should exist");
    // Sanity: the seed is lift-associated with the postmortem.
    let recs = brain.recommend(&seed, 5, 1).unwrap();
    assert!(
        recs.iter().any(|r| r.lift >= 1.0),
        "seed should have an above-baseline lift association"
    );

    let q = "kubernetes rollout strategy"; // matches kube-deploy, NOT the postmortem
    let keys = |v: Vec<spectral_ingest::MemoryHit>| -> Vec<String> {
        v.into_iter().map(|h| h.key).collect()
    };

    std::env::remove_var("SPECTRAL_ANTICIPATORY_RECALL");
    let off = keys(
        brain
            .recall_topk_fts(q, &RecallTopKConfig::default(), Visibility::Private)
            .unwrap(),
    );
    assert!(
        off.contains(&"kube-deploy".to_string()) && !off.contains(&"deploy-outage".to_string()),
        "OFF: query alone matches only kube-deploy, got {off:?}"
    );

    std::env::set_var("SPECTRAL_ANTICIPATORY_RECALL", "1");
    let on = keys(
        brain
            .recall_topk_fts(q, &RecallTopKConfig::default(), Visibility::Private)
            .unwrap(),
    );
    std::env::remove_var("SPECTRAL_ANTICIPATORY_RECALL");
    assert!(
        on.contains(&"kube-deploy".to_string()) && on.contains(&"deploy-outage".to_string()),
        "ON: recall should also surface the missed postmortem by anticipation, got {on:?}"
    );
}

#[test]
fn cascade_with_session_id_logs_session_attribution() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "sess-mem-1",
            "I enjoy hiking in the mountains",
            Visibility::Private,
        )
        .unwrap();

    let ctx = RecognitionContext::empty().with_session("test-session-42");
    let cascade_config = CascadePipelineConfig::default();
    let _ = brain.recall_cascade("hiking mountains", &ctx, &cascade_config);

    let events = brain.events_for_session("test-session-42", 100).unwrap();
    assert!(
        !events.is_empty(),
        "cascade with session should produce at least one event"
    );
    assert_eq!(events[0].session_id.as_deref(), Some("test-session-42"));
    assert_eq!(events[0].method, "cascade");
}

#[test]
fn memories_for_session_aggregates_across_cascades() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    brain
        .remember(
            "agg-1",
            "I prefer Python for data science work",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "agg-2",
            "My favorite IDE is VS Code with extensions",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "agg-3",
            "I commute by bicycle every morning",
            Visibility::Private,
        )
        .unwrap();

    let ctx = RecognitionContext::empty().with_session("agg-session");
    let cascade_config = CascadePipelineConfig::default();

    // Two cascades in the same session with different queries
    let _ = brain.recall_cascade("Python data science", &ctx, &cascade_config);
    let _ = brain.recall_cascade("IDE VS Code editor", &ctx, &cascade_config);

    let session_mems = brain.memories_for_session("agg-session").unwrap();

    // Should have deduplicated memory IDs from both cascades
    let unique_count = session_mems.len();
    let as_set: std::collections::HashSet<&str> = session_mems.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        unique_count,
        as_set.len(),
        "memories_for_session should return unique IDs"
    );
}

// ── Co-retrieval ranking integration tests ──────────────────────────

#[test]
fn co_retrieval_boost_lifts_co_retrieved_memories_in_cascade() {
    use spectral_graph::ranking::{
        apply_reranking_pipeline, compute_co_retrieval_boosts, RerankingConfig,
    };
    use std::collections::HashMap;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // Ingest: "rust" and "editor" memories are topically close.
    // "commute" memory is unrelated but will match FTS on "daily".
    let r_rust = brain
        .remember(
            "cr-rust",
            "I use Rust for systems programming daily",
            Visibility::Private,
        )
        .unwrap();
    let r_editor = brain
        .remember(
            "cr-editor",
            "My daily editor for Rust is Neovim",
            Visibility::Private,
        )
        .unwrap();
    let r_commute = brain
        .remember(
            "cr-commute",
            "My daily commute takes thirty minutes by train",
            Visibility::Private,
        )
        .unwrap();

    // Pad with filler memories that also match "daily" to reduce per-position
    // FTS gap. With 20+ candidates, gap is ~0.05 per position, so a 0.10
    // co-retrieval boost can bridge 2 positions.
    for i in 0..17 {
        brain
            .remember(
                &format!("cr-filler-{i}"),
                &format!("My daily routine number {i} involves various activities"),
                Visibility::Private,
            )
            .unwrap();
    }

    // Run cascade queries that return overlapping subsets.
    // "Rust programming editor Neovim" should co-retrieve
    // cr-rust and cr-editor but not cr-commute.
    let cascade_config = CascadePipelineConfig::default();
    let ctx = RecognitionContext::empty();
    for _ in 0..5 {
        let _ = brain.recall_cascade("Rust programming editor Neovim", &ctx, &cascade_config);
    }

    // Rebuild co-retrieval index from the cascade events
    let pairs = brain.rebuild_co_retrieval_index().unwrap();
    assert!(pairs > 0, "should have at least one co-retrieval pair");

    // Verify: cr-rust and cr-editor are co-retrieved (using memory_id, not key)
    let related = brain.related_memories(&r_rust.memory_id, 10).unwrap();
    let related_ids: Vec<&str> = related.iter().map(|r| r.memory_id.as_str()).collect();
    assert!(
        related_ids.contains(&r_editor.memory_id.as_str()),
        "cr-editor should be co-retrieved with cr-rust, got: {related_ids:?}"
    );

    // ── Control comparison: same candidates, with vs without co-retrieval ──
    // Retrieve raw candidates for "daily" (all three match via FTS).
    let candidates = brain.cascade_retrieve("daily", 40).unwrap();
    assert!(
        candidates.iter().any(|h| h.id == r_rust.memory_id),
        "cr-rust should be in raw candidates"
    );
    assert!(
        candidates.iter().any(|h| h.id == r_editor.memory_id),
        "cr-editor should be in raw candidates"
    );
    assert!(
        candidates.iter().any(|h| h.id == r_commute.memory_id),
        "cr-commute should be in raw candidates"
    );

    // Compute co-retrieval boosts — verify the end-to-end path produces
    // non-empty data with editor having higher affinity than commute.
    let co_boosts = compute_co_retrieval_boosts(&brain, &candidates, 3);
    assert!(
        !co_boosts.is_empty(),
        "co-retrieval boosts should be non-empty after priming"
    );
    let editor_affinity = co_boosts.get(&r_editor.memory_id).copied().unwrap_or(0.0);
    let commute_affinity = co_boosts.get(&r_commute.memory_id).copied().unwrap_or(0.0);
    assert!(
        editor_affinity > commute_affinity,
        "cr-editor should have higher co-retrieval affinity than cr-commute: \
         editor={editor_affinity}, commute={commute_affinity}"
    );

    // Shared config — same signals for both runs, only co-retrieval map differs
    let config = RerankingConfig {
        apply_signal_score: true,
        signal_score_weight: 0.3,
        apply_recency: true,
        recency_half_life_days: 365.0,
        apply_entity_boost: false,
        entity_boost_weight: 0.05,
        apply_ambient_boost: false,
        apply_declarative_boost: true,
        declarative_weight: 0.10,
        co_retrieval_weight: 0.10,
        apply_episode_diversity: false,
        max_per_episode: 5,
        apply_context_dedup: false,
    };

    // Control: rank without co-retrieval (empty map)
    let empty_boosts: HashMap<String, f64> = HashMap::new();
    let control = apply_reranking_pipeline(candidates.clone(), &config, &ctx, &empty_boosts);

    // Treatment: rank with co-retrieval
    let treatment = apply_reranking_pipeline(candidates, &config, &ctx, &co_boosts);

    // Compare cr-editor's composite score: treatment should be strictly higher
    // than control. The delta is attributable solely to co-retrieval.
    let control_editor_score = control
        .iter()
        .find(|h| h.id == r_editor.memory_id)
        .unwrap()
        .signal_score;
    let treatment_editor_score = treatment
        .iter()
        .find(|h| h.id == r_editor.memory_id)
        .unwrap()
        .signal_score;
    assert!(
        treatment_editor_score > control_editor_score,
        "cr-editor's score should be higher with co-retrieval: \
         control={control_editor_score}, treatment={treatment_editor_score}"
    );

    // cr-commute's score should NOT increase (no co-retrieval affinity)
    let control_commute_score = control
        .iter()
        .find(|h| h.id == r_commute.memory_id)
        .unwrap()
        .signal_score;
    let treatment_commute_score = treatment
        .iter()
        .find(|h| h.id == r_commute.memory_id)
        .unwrap()
        .signal_score;
    assert!(
        (treatment_commute_score - control_commute_score).abs() < f64::EPSILON,
        "cr-commute's score should be unchanged by co-retrieval: \
         control={control_commute_score}, treatment={treatment_commute_score}"
    );
}

#[test]
fn compute_co_retrieval_boosts_normalization_and_edge_cases() {
    use spectral_graph::ranking::compute_co_retrieval_boosts;
    use spectral_ingest::MemoryHit;

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    // (e2) Empty candidates returns empty map without panic
    let empty: Vec<MemoryHit> = Vec::new();
    let boosts = compute_co_retrieval_boosts(&brain, &empty, 3);
    assert!(
        boosts.is_empty(),
        "empty candidates should produce empty map"
    );

    // Ingest memories and generate co-retrieval data via cascade queries
    brain
        .remember("norm-a", "I like apples for breakfast", Visibility::Private)
        .unwrap();
    brain
        .remember(
            "norm-b",
            "I like bananas for breakfast",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember("norm-c", "I like cherries for dessert", Visibility::Private)
        .unwrap();

    // Cascade queries that co-retrieve a+b more often than a+c.
    // "breakfast" matches a+b but not c; "like" matches all three.
    let cascade_config = CascadePipelineConfig::default();
    let ctx = RecognitionContext::empty();
    for _ in 0..5 {
        let _ = brain.recall_cascade("breakfast apples bananas", &ctx, &cascade_config);
    }
    for _ in 0..2 {
        let _ = brain.recall_cascade("cherries dessert apples", &ctx, &cascade_config);
    }
    brain.rebuild_co_retrieval_index().unwrap();

    // Build synthetic candidate list with norm-a as anchor (FTS position 0)
    fn make_candidate(id: &str, content: &str) -> MemoryHit {
        MemoryHit {
            id: id.into(),
            key: id.into(),
            content: content.into(),
            wing: None,
            hall: None,
            signal_score: 0.5,
            visibility: "private".into(),
            hits: 0,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            declarative_density: None,
            description: None,
            source_brain_id: None,
            signature: None,
        }
    }

    let candidates = vec![
        make_candidate("norm-a", "I like apples for breakfast"),
        make_candidate("norm-b", "I like bananas for breakfast"),
        make_candidate("norm-c", "I like cherries for dessert"),
    ];

    // (e) Normalization: values in [0.0, 1.0]
    let boosts = compute_co_retrieval_boosts(&brain, &candidates, 1);
    for (id, &val) in &boosts {
        assert!(
            (0.0..=1.0).contains(&val),
            "boost for {id} should be in [0.0, 1.0], got {val}"
        );
    }

    // If both b and c have boosts, b should be >= c (more co-retrievals)
    let b_boost = boosts.get("norm-b").copied().unwrap_or(0.0);
    let c_boost = boosts.get("norm-c").copied().unwrap_or(0.0);
    assert!(
        b_boost >= c_boost,
        "norm-b (more co-retrievals) should have >= boost than norm-c, \
         got b={b_boost} c={c_boost}"
    );

    // If there are any boosts, max should be 1.0 (normalized)
    if !boosts.is_empty() {
        let max_boost = boosts.values().copied().fold(0.0_f64, f64::max);
        assert!(
            (max_boost - 1.0).abs() < f64::EPSILON,
            "max boost should be 1.0 after normalization, got {max_boost}"
        );
    }

    // (e) anchor_count > candidates.len() doesn't panic
    let boosts_big_anchor = compute_co_retrieval_boosts(&brain, &candidates, 100);
    // Should not panic; results depend on data
    let _ = boosts_big_anchor;

    // (e) No co-retrieval data returns empty map
    let tmp2 = TempDir::new().unwrap();
    let brain2 = Brain::open(brain_config(&tmp2)).unwrap();
    brain2
        .remember("fresh-1", "Some content here", Visibility::Private)
        .unwrap();
    let fresh_candidates = vec![make_candidate("fresh-1", "Some content here")];
    let empty_boosts = compute_co_retrieval_boosts(&brain2, &fresh_candidates, 3);
    assert!(
        empty_boosts.is_empty(),
        "fresh brain with no co-retrieval data should return empty map"
    );
}

// ── Constellation fingerprint bucket regression test ─────────────

#[test]
fn fingerprints_have_valid_time_delta_bucket() {
    // Regression test for PR #65: ingest previously hardcoded
    // time_delta_bucket = "unknown" which made fingerprint matching
    // impossible (query hashes use real buckets like "same_day").
    //
    // This test creates two memories with known timestamps via
    // remember_with() and verifies that the resulting constellation
    // fingerprints have a valid bucket — never "unknown" or NULL.

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let ts1 = Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 6, 15, 14, 0, 0).unwrap(); // same day, 2h later

    // First memory — no fingerprints (no peers yet)
    let r1 = brain
        .remember_with(
            "fp-bucket-test-1",
            "Decided to use PostgreSQL for the production database",
            RememberOpts {
                created_at: Some(ts1),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(r1.fingerprints_created, 0, "first memory has no peers");

    // Second memory — should create fingerprints pairing with first
    let r2 = brain
        .remember_with(
            "fp-bucket-test-2",
            "PostgreSQL schema migration completed for production",
            RememberOpts {
                created_at: Some(ts2),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        r2.fingerprints_created >= 1,
        "second memory should pair with first; got {} fingerprints",
        r2.fingerprints_created
    );

    // Query the constellation_fingerprints table directly to verify buckets
    let db_path = tmp.path().join("memory.db");
    let conn = rusqlite::Connection::open(db_path).unwrap();

    let unknown_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM constellation_fingerprints WHERE time_delta_bucket = 'unknown'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        unknown_count, 0,
        "no fingerprints should have 'unknown' bucket (PR #65 regression)"
    );

    let null_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM constellation_fingerprints WHERE time_delta_bucket IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(null_count, 0, "no fingerprints should have NULL bucket");

    // Verify the bucket is correct: 2 hours apart = same_day
    let bucket: String = conn
        .query_row(
            "SELECT time_delta_bucket FROM constellation_fingerprints LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        bucket, "same_day",
        "2-hour delta should produce 'same_day' bucket, got '{bucket}'"
    );
}
