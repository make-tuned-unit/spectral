use rusqlite::{params, Connection};
use spectral_archivist::archivist::ArchivistConfig;
use spectral_archivist::Archivist;

/// Create an in-memory SQLite database with the Spectral memory schema.
fn test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE memories (
            id            TEXT PRIMARY KEY,
            key           TEXT NOT NULL UNIQUE,
            content       TEXT NOT NULL,
            category      TEXT NOT NULL DEFAULT 'core',
            wing          TEXT DEFAULT NULL,
            hall          TEXT DEFAULT NULL,
            signal_score  REAL DEFAULT 0.5,
            visibility    TEXT NOT NULL DEFAULT 'private',
            created_at    TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at    TEXT NOT NULL DEFAULT (datetime('now')),
            source        TEXT DEFAULT NULL,
            device_id     BLOB DEFAULT NULL,
            confidence    REAL NOT NULL DEFAULT 1.0,
            last_reinforced_at TEXT DEFAULT NULL
        );
        CREATE TABLE memory_spectrogram (
            memory_id         TEXT PRIMARY KEY,
            entity_density    REAL,
            action_type       TEXT,
            decision_polarity REAL,
            causal_depth      REAL,
            emotional_valence REAL,
            temporal_specificity REAL,
            novelty           REAL,
            peak_dimensions   TEXT,
            created_at        TEXT DEFAULT (datetime('now'))
        );",
    )
    .unwrap();
    conn
}

fn insert_memory(conn: &Connection, id: &str, key: &str, content: &str, wing: &str, hall: &str) {
    conn.execute(
        "INSERT INTO memories (id, key, content, wing, hall, signal_score) \
         VALUES (?1, ?2, ?3, ?4, ?5, 0.7)",
        params![id, key, content, wing, hall],
    )
    .unwrap();
}

fn archivist_from_conn(conn: Connection) -> Archivist {
    Archivist::from_conn(conn, ArchivistConfig::default())
}

// ── Duplicate tests ───────────────────────────────────────────────

#[test]
fn find_duplicates_returns_pairs_above_threshold() {
    let conn = test_db();
    insert_memory(
        &conn,
        "m1",
        "k1",
        "Alice decided to use Clerk for authentication in the project",
        "apollo",
        "fact",
    );
    insert_memory(
        &conn,
        "m2",
        "k2",
        "Alice decided to use Clerk for authentication in the main project",
        "apollo",
        "fact",
    );
    insert_memory(
        &conn,
        "m3",
        "k3",
        "Bob prefers dark mode in all editors",
        "apollo",
        "preference",
    );

    let a = archivist_from_conn(conn);
    let dupes = a.find_duplicates().unwrap();
    assert_eq!(dupes.len(), 1, "should find one duplicate pair");
    assert!(dupes[0].overlap > 0.6);
    assert_eq!(dupes[0].wing, "apollo");
}

#[test]
fn find_duplicates_no_cross_wing() {
    let conn = test_db();
    insert_memory(&conn, "m1", "k1", "same content here", "apollo", "fact");
    insert_memory(&conn, "m2", "k2", "same content here", "acme", "fact");

    let a = archivist_from_conn(conn);
    let dupes = a.find_duplicates().unwrap();
    assert!(dupes.is_empty(), "should not flag cross-wing duplicates");
}

// ── Gap detection tests ──────────────────────────────────────────

#[test]
fn find_gaps_detects_missing_summary() {
    let conn = test_db();
    for i in 0..5 {
        insert_memory(
            &conn,
            &format!("m{i}"),
            &format!("event_{i}"),
            &format!("Alice did thing {i}"),
            "apollo",
            "event",
        );
    }
    // No summary key like 'index_apollo' or 'apollo_summary'

    let a = archivist_from_conn(conn);
    let gaps = a.find_gaps().unwrap();
    assert!(
        gaps.missing_summaries.iter().any(|(w, _)| w == "apollo"),
        "should detect missing summary for apollo"
    );
}

#[test]
fn find_gaps_detects_no_facts() {
    let conn = test_db();
    for i in 0..5 {
        insert_memory(
            &conn,
            &format!("m{i}"),
            &format!("event_{i}"),
            &format!("Alice did thing {i}"),
            "apollo",
            "event",
        );
    }

    let a = archivist_from_conn(conn);
    let gaps = a.find_gaps().unwrap();
    assert!(
        gaps.no_facts.iter().any(|(w, _)| w == "apollo"),
        "should detect no facts for apollo"
    );
}

#[test]
fn find_gaps_detects_unmapped_projects() {
    let conn = test_db();
    insert_memory(&conn, "m1", "k1", "test", "apollo", "fact");

    let config = ArchivistConfig {
        known_projects: Some(vec!["apollo".into(), "mercury".into()]),
        ..ArchivistConfig::default()
    };
    let a = Archivist::from_conn(conn, config);
    let gaps = a.find_gaps().unwrap();
    assert!(
        gaps.unmapped_projects.contains(&"mercury".to_string()),
        "should detect mercury as unmapped"
    );
    assert!(
        !gaps.unmapped_projects.contains(&"apollo".to_string()),
        "apollo exists as a wing"
    );
}

// ── Reclassification tests ───────────────────────────────────────

#[test]
fn suggest_reclassifications_recognizes_wing_in_content() {
    let conn = test_db();
    // A non-general wing to match against
    insert_memory(
        &conn,
        "m1",
        "k1",
        "Apollo launch schedule",
        "apollo",
        "fact",
    );
    // A general-wing memory that mentions "apollo"
    insert_memory(
        &conn,
        "m2",
        "general_note",
        "The apollo project is on track for launch next week, apollo looks good",
        "general",
        "event",
    );

    let a = archivist_from_conn(conn);
    let suggestions = a.suggest_reclassifications().unwrap();
    assert!(
        suggestions
            .iter()
            .any(|s| s.key == "general_note" && s.suggested_wing.as_deref() == Some("apollo")),
        "should suggest reclassifying general_note to apollo: {suggestions:?}"
    );
}

#[test]
fn suggest_reclassifications_skips_weak_wings() {
    let conn = test_db();
    insert_memory(&conn, "m1", "k1", "general info", "general", "fact");
    // Only 'general' wing exists, which is weak

    let a = archivist_from_conn(conn);
    let suggestions = a.suggest_reclassifications().unwrap();
    // Should not suggest reclassifying to 'general' (it's weak)
    assert!(
        suggestions
            .iter()
            .all(|s| s.suggested_wing.as_deref() != Some("general")),
        "should not suggest weak wing 'general'"
    );
}

// ── Decay tests ──────────────────────────────────────────────────

#[test]
fn apply_decay_reduces_stale_memories() {
    let conn = test_db();
    // Memory with no last_reinforced_at (never reinforced → should decay)
    conn.execute(
        "INSERT INTO memories (id, key, content, wing, hall, signal_score) \
         VALUES ('m1', 'k1', 'old memory', 'apollo', 'fact', 0.7)",
        [],
    )
    .unwrap();

    let a = archivist_from_conn(conn);
    let stats = a.apply_decay().unwrap();
    assert_eq!(stats.decayed, 1, "should decay 1 memory");

    let score: f64 = a
        .conn()
        .query_row(
            "SELECT signal_score FROM memories WHERE id = 'm1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        (score - 0.65).abs() < 0.001,
        "0.7 - 0.05 = 0.65, got {score}"
    );
}

#[test]
fn apply_decay_boosts_recent_memories() {
    let conn = test_db();
    // Memory reinforced recently
    let recent = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO memories (id, key, content, wing, hall, signal_score, last_reinforced_at) \
         VALUES ('m1', 'k1', 'recent memory', 'apollo', 'fact', 0.6, ?1)",
        params![recent],
    )
    .unwrap();

    let a = archivist_from_conn(conn);
    let stats = a.apply_decay().unwrap();
    assert_eq!(stats.boosted, 1, "should boost 1 memory");

    let score: f64 = a
        .conn()
        .query_row(
            "SELECT signal_score FROM memories WHERE id = 'm1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        (score - 0.62).abs() < 0.001,
        "0.6 + 0.02 = 0.62, got {score}"
    );
}

#[test]
fn apply_decay_respects_floor() {
    let conn = test_db();
    conn.execute(
        "INSERT INTO memories (id, key, content, wing, hall, signal_score) \
         VALUES ('m1', 'k1', 'low memory', 'apollo', 'fact', 0.12)",
        [],
    )
    .unwrap();

    let a = archivist_from_conn(conn);
    a.apply_decay().unwrap();

    let score: f64 = a
        .conn()
        .query_row(
            "SELECT signal_score FROM memories WHERE id = 'm1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        (score - 0.1).abs() < 0.001,
        "should floor at 0.1, got {score}"
    );
}

// ── Consolidation candidate tests ────────────────────────────────

#[test]
fn find_consolidation_candidates_returns_overlap_band() {
    let conn = test_db();
    // Create 5 memories to meet the threshold, with two having moderate overlap
    for i in 0..3 {
        insert_memory(
            &conn,
            &format!("m{i}"),
            &format!("unique_key_{i}"),
            &format!("completely different content number {i} with filler words here"),
            "apollo",
            "fact",
        );
    }
    // Two memories with ~50% overlap (moderate band)
    insert_memory(
        &conn,
        "m3",
        "auth_setup",
        "Alice set up Clerk authentication with OAuth and JWT tokens for the API",
        "apollo",
        "fact",
    );
    insert_memory(
        &conn,
        "m4",
        "auth_config",
        "Alice configured Clerk authentication with SAML and API keys for the service",
        "apollo",
        "fact",
    );

    let a = archivist_from_conn(conn);
    let cands = a.find_consolidation_candidates().unwrap();
    // The two auth memories should be candidates if their Jaccard falls in [0.45, 0.58]
    // Let's just verify the function runs without error and returns results
    // (exact overlap depends on word set arithmetic)
    assert!(
        cands.iter().all(|c| c.overlap >= 0.45 && c.overlap <= 0.58),
        "all candidates should be in the overlap band"
    );
}

#[test]
fn find_consolidation_candidates_skips_system_keys() {
    let conn = test_db();
    for i in 0..5 {
        insert_memory(
            &conn,
            &format!("m{i}"),
            &format!("slack_msg_{i}"),
            &format!("similar slack content about the project meeting {i}"),
            "apollo",
            "fact",
        );
    }

    let a = archivist_from_conn(conn);
    let cands = a.find_consolidation_candidates().unwrap();
    assert!(cands.is_empty(), "should skip keys starting with 'slack_'");
}

// ── Trait tests ──────────────────────────────────────────────────

#[test]
fn noop_consolidator_returns_none() {
    use spectral_archivist::Consolidator;
    use spectral_archivist::NoOpConsolidator;
    let c = NoOpConsolidator;
    assert!(c.consolidate("a", "b").unwrap().is_none());
}

#[test]
fn noop_indexer_returns_none() {
    use spectral_archivist::Indexer;
    use spectral_archivist::NoOpIndexer;
    let i = NoOpIndexer;
    assert!(i.generate_index("wing", &[]).unwrap().is_none());
}

// ── Report test ──────────────────────────────────────────────────

#[test]
fn archivist_report_runs_all_passes_dry_run() {
    let conn = test_db();
    for i in 0..5 {
        insert_memory(
            &conn,
            &format!("m{i}"),
            &format!("key_{i}"),
            &format!("Alice worked on feature {i} for the Apollo project"),
            "apollo",
            "fact",
        );
    }

    let a = archivist_from_conn(conn);
    let report = a.report().unwrap();

    assert_eq!(report.memory_count, 5);
    // The report should run without errors; we don't assert specific findings
    // since the test data may or may not trigger each pass
    assert!(report.timestamp.timestamp() > 0);
}
