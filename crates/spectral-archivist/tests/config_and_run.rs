//! `ArchivistConfig` thresholds, and the mutating `run()` orchestration.
//!
//! The existing suite covers each pass at its default settings. What it does
//! not cover is whether the *config* reaches those passes — a threshold that
//! is silently ignored produces a plausible-looking report computed with the
//! wrong parameters, and nothing about the output says so.
//!
//! Each test below therefore changes one setting and asserts the output
//! CHANGES with it, rather than asserting a fixed result at a fixed setting.
//! It also covers `run()`, which is the only entry point that mutates and had
//! no caller in the suite, and the boost ceiling (the floor was already
//! pinned; its counterpart was not).

use rusqlite::{params, Connection};
use spectral_archivist::archivist::{Archivist, ArchivistConfig};
use spectral_archivist::traits::{Consolidator, Indexer};

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
            memory_id            TEXT PRIMARY KEY,
            entity_density       REAL,
            action_type          TEXT,
            decision_polarity    REAL,
            causal_depth         REAL,
            emotional_valence    REAL,
            temporal_specificity REAL,
            novelty              REAL,
            peak_dimensions      TEXT,
            created_at           TEXT DEFAULT (datetime('now'))
        );",
    )
    .unwrap();
    conn
}

fn insert(conn: &Connection, id: &str, key: &str, content: &str, wing: &str, hall: &str) {
    conn.execute(
        "INSERT INTO memories (id, key, content, wing, hall, signal_score) \
         VALUES (?1, ?2, ?3, ?4, ?5, 0.7)",
        params![id, key, content, wing, hall],
    )
    .unwrap();
}

/// Insert with an explicit score and reinforcement age (days ago, or never).
fn insert_aged(conn: &Connection, id: &str, key: &str, score: f64, days_ago: Option<i64>) {
    let reinforced = days_ago.map(|d| format!("-{d} days"));
    match reinforced {
        Some(offset) => conn.execute(
            "INSERT INTO memories (id, key, content, wing, hall, signal_score, \
             last_reinforced_at) VALUES (?1, ?2, 'content', 'w', 'fact', ?3, \
             datetime('now', ?4))",
            params![id, key, score, offset],
        ),
        None => conn.execute(
            "INSERT INTO memories (id, key, content, wing, hall, signal_score, \
             last_reinforced_at) VALUES (?1, ?2, 'content', 'w', 'fact', ?3, NULL)",
            params![id, key, score],
        ),
    }
    .unwrap();
}

fn score_of(conn: &Connection, key: &str) -> f64 {
    conn.query_row(
        "SELECT signal_score FROM memories WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .unwrap()
}

/// Consolidation only considers wings with >= 5 memories, so a fixture needs
/// padding. The fillers share no words with anything, so they never form a
/// candidate pair themselves.
fn pad_wing(conn: &Connection, wing: &str, hall: &str, n: usize) {
    for i in 0..n {
        insert(
            conn,
            &format!("pad{i}-{wing}"),
            &format!("pad-{wing}-{i}"),
            &format!("unrelated{i} filler{i} words{i}"),
            wing,
            hall,
        );
    }
}

/// Does the candidate list contain the (a, b) pair, in either order?
fn has_pair(
    cands: &[spectral_archivist::candidates::ConsolidationCandidate],
    a: &str,
    b: &str,
) -> bool {
    cands
        .iter()
        .any(|c| (c.key_a == a && c.key_b == b) || (c.key_a == b && c.key_b == a))
}

fn archivist(conn: Connection, config: ArchivistConfig) -> Archivist {
    Archivist::from_conn(conn, config)
}

// ── config thresholds must reach their pass ────────────────────────

/// `duplicate_threshold` must reach the duplicate pass. Two memories with
/// partial overlap are duplicates at a low threshold and not at a high one, so
/// the same data yields different reports.
#[test]
fn duplicate_threshold_changes_what_counts_as_a_duplicate() {
    let make = || {
        let conn = test_db();
        insert(&conn, "1", "a", "alpha beta gamma delta", "w", "fact");
        insert(&conn, "2", "b", "alpha beta gamma epsilon", "w", "fact");
        conn
    };

    let strict = archivist(
        make(),
        ArchivistConfig {
            duplicate_threshold: 0.99,
            ..Default::default()
        },
    );
    let loose = archivist(
        make(),
        ArchivistConfig {
            duplicate_threshold: 0.1,
            ..Default::default()
        },
    );

    assert!(
        strict.find_duplicates().unwrap().is_empty(),
        "a 0.99 threshold should reject a partial overlap"
    );
    assert!(
        !loose.find_duplicates().unwrap().is_empty(),
        "a 0.1 threshold should accept the same partial overlap — the \
         threshold is not reaching the duplicate pass"
    );
}

/// The consolidation band is a *window*: too-similar pairs are duplicates, not
/// consolidation candidates. Narrowing the window must exclude a pair that a
/// wide window admits.
#[test]
fn the_consolidation_overlap_band_bounds_candidates_at_both_ends() {
    // Jaccard("alpha beta gamma delta epsilon", "alpha beta gamma zeta eta")
    // = 3/7 = 0.43, so a wide band admits it and either narrow band excludes it.
    let make = || {
        let conn = test_db();
        insert(
            &conn,
            "1",
            "a",
            "alpha beta gamma delta epsilon",
            "w",
            "fact",
        );
        insert(&conn, "2", "b", "alpha beta gamma zeta eta", "w", "fact");
        pad_wing(&conn, "w", "fact", 3);
        conn
    };

    let wide = archivist(
        make(),
        ArchivistConfig {
            consolidation_overlap_min: 0.01,
            consolidation_overlap_max: 0.99,
            ..Default::default()
        },
    );
    let above = archivist(
        make(),
        ArchivistConfig {
            consolidation_overlap_min: 0.95,
            consolidation_overlap_max: 0.99,
            ..Default::default()
        },
    );
    let below = archivist(
        make(),
        ArchivistConfig {
            consolidation_overlap_min: 0.001,
            consolidation_overlap_max: 0.002,
            ..Default::default()
        },
    );

    assert!(
        has_pair(&wide.find_consolidation_candidates().unwrap(), "a", "b"),
        "a wide band should admit the pair"
    );
    assert!(
        !has_pair(&above.find_consolidation_candidates().unwrap(), "a", "b"),
        "a band above the pair's overlap should exclude it (min not applied)"
    );
    assert!(
        !has_pair(&below.find_consolidation_candidates().unwrap(), "a", "b"),
        "a band below the pair's overlap should exclude it (max not applied)"
    );
}

/// `consolidation_skip_prefixes` and `consolidation_skip_contains` are the
/// caller's way of excluding machine-generated keys. Emptying them must let a
/// previously-skipped pair through.
#[test]
fn the_consolidation_skip_lists_are_honoured_and_configurable() {
    let make = || {
        let conn = test_db();
        insert(
            &conn,
            "1",
            "slack_a",
            "alpha beta gamma delta epsilon",
            "w",
            "fact",
        );
        insert(
            &conn,
            "2",
            "slack_b",
            "alpha beta gamma zeta eta",
            "w",
            "fact",
        );
        pad_wing(&conn, "w", "fact", 3);
        conn
    };

    let wide_band = ArchivistConfig {
        consolidation_overlap_min: 0.01,
        consolidation_overlap_max: 0.99,
        ..Default::default()
    };

    let with_skips = archivist(make(), wide_band.clone());
    assert!(
        !has_pair(
            &with_skips.find_consolidation_candidates().unwrap(),
            "slack_a",
            "slack_b"
        ),
        "the default slack_ prefix skip did not apply"
    );

    let without_skips = archivist(
        make(),
        ArchivistConfig {
            consolidation_skip_prefixes: vec![],
            consolidation_skip_contains: vec![],
            ..wide_band
        },
    );
    assert!(
        has_pair(
            &without_skips.find_consolidation_candidates().unwrap(),
            "slack_a",
            "slack_b"
        ),
        "clearing the skip lists did not admit the pair — the lists are not \
         reaching the pass"
    );
}

// ── decay and boost configuration ──────────────────────────────────

/// `decay_amount` must reach the decay pass: a larger amount removes more.
#[test]
fn decay_amount_controls_how_much_is_removed() {
    let make = || {
        let conn = test_db();
        insert_aged(&conn, "1", "stale", 0.9, Some(90));
        conn
    };

    let small = archivist(
        make(),
        ArchivistConfig {
            decay_amount: 0.01,
            ..Default::default()
        },
    );
    small.apply_decay().unwrap();
    let after_small = score_of(small.conn(), "stale");

    let large = archivist(
        make(),
        ArchivistConfig {
            decay_amount: 0.5,
            ..Default::default()
        },
    );
    large.apply_decay().unwrap();
    let after_large = score_of(large.conn(), "stale");

    assert!(
        after_large < after_small,
        "a larger decay_amount did not remove more: {after_large} vs {after_small}"
    );
}

/// `decay_threshold_days` decides WHICH memories are stale. A memory 10 days
/// idle decays under a 5-day threshold and survives a 30-day one.
#[test]
fn decay_threshold_days_decides_what_counts_as_stale() {
    let make = || {
        let conn = test_db();
        insert_aged(&conn, "1", "m", 0.9, Some(10));
        conn
    };

    let patient = archivist(
        make(),
        ArchivistConfig {
            decay_threshold_days: 30,
            boost_threshold_days: 0,
            ..Default::default()
        },
    );
    let stats = patient.apply_decay().unwrap();
    assert_eq!(
        stats.decayed, 0,
        "a 10-day-idle memory decayed under a 30-day threshold"
    );

    let impatient = archivist(
        make(),
        ArchivistConfig {
            decay_threshold_days: 5,
            boost_threshold_days: 0,
            ..Default::default()
        },
    );
    assert_eq!(
        impatient.apply_decay().unwrap().decayed,
        1,
        "a 10-day-idle memory did not decay under a 5-day threshold"
    );
}

/// The boost ceiling. The floor is already pinned by the existing suite; its
/// counterpart was not, and an unbounded boost would let scores exceed the
/// range every consumer assumes.
#[test]
fn boost_is_capped_at_max_signal_score() {
    let conn = test_db();
    insert_aged(&conn, "1", "hot", 0.99, Some(1));
    let a = archivist(
        conn,
        ArchivistConfig {
            boost_amount: 0.5,
            max_signal_score: 1.0,
            ..Default::default()
        },
    );

    let stats = a.apply_decay().unwrap();
    assert_eq!(
        stats.boosted, 1,
        "a recently reinforced memory was not boosted"
    );
    let score = score_of(a.conn(), "hot");
    assert!(score <= 1.0, "boost exceeded max_signal_score: {score}");
    assert!(
        (score - 1.0).abs() < 1e-9,
        "expected the ceiling exactly, got {score}"
    );
}

/// A custom ceiling below 1.0 must be respected too, so the cap is the config
/// value and not a hardcoded 1.0.
#[test]
fn a_custom_ceiling_is_respected() {
    let conn = test_db();
    insert_aged(&conn, "1", "hot", 0.5, Some(1));
    let a = archivist(
        conn,
        ArchivistConfig {
            boost_amount: 0.9,
            max_signal_score: 0.6,
            ..Default::default()
        },
    );
    a.apply_decay().unwrap();
    let score = score_of(a.conn(), "hot");
    assert!(
        (score - 0.6).abs() < 1e-9,
        "expected the custom ceiling 0.6, got {score}"
    );
}

/// Decay and boost are disjoint: a memory cannot be both stale and recent, so
/// no memory may be counted in both buckets in one pass.
#[test]
fn a_single_pass_never_both_decays_and_boosts_the_same_memory() {
    let conn = test_db();
    insert_aged(&conn, "1", "stale", 0.9, Some(90));
    insert_aged(&conn, "2", "hot", 0.2, Some(1));
    insert_aged(&conn, "3", "never", 0.9, None);
    let a = archivist(conn, ArchivistConfig::default());

    let stats = a.apply_decay().unwrap();
    // stale + never-reinforced decay; hot boosts.
    assert_eq!(
        stats.decayed, 2,
        "expected the stale and never-reinforced rows"
    );
    assert_eq!(
        stats.boosted, 1,
        "expected only the recently reinforced row"
    );
    assert!(score_of(a.conn(), "stale") < 0.9);
    assert!(
        score_of(a.conn(), "never") < 0.9,
        "a never-reinforced memory should decay"
    );
    assert!(score_of(a.conn(), "hot") > 0.2);
}

// ── run(): the mutating orchestration ──────────────────────────────

/// `run()` is the only entry point that both reports and mutates, and nothing
/// in the suite called it. It must do BOTH: return the full report and leave
/// the decay applied.
#[test]
fn run_reports_and_mutates_in_one_pass() {
    let conn = test_db();
    insert(&conn, "1", "a", "alpha beta gamma delta", "w", "fact");
    insert(&conn, "2", "b", "alpha beta gamma epsilon", "w", "fact");
    insert_aged(&conn, "3", "stale", 0.9, Some(90));
    let a = archivist(
        conn,
        ArchivistConfig {
            duplicate_threshold: 0.1,
            ..Default::default()
        },
    );

    let before = score_of(a.conn(), "stale");
    let run = a.run().unwrap();

    assert_eq!(
        run.report.memory_count, 3,
        "the report should count every memory"
    );
    assert!(
        !run.report.duplicates.is_empty(),
        "run() should include the duplicate pass"
    );
    // All three rows are stale (the two duplicates were inserted with a NULL
    // last_reinforced_at, which counts as never reinforced), so all three decay.
    assert_eq!(run.decay_stats.decayed, 3, "run() should apply decay");
    assert!(
        score_of(a.conn(), "stale") < before,
        "run() reported a decay it did not actually apply"
    );
}

/// `report()` by contrast must be a dry run — it must not change any score.
#[test]
fn report_is_a_dry_run_and_mutates_nothing() {
    let conn = test_db();
    insert_aged(&conn, "1", "stale", 0.9, Some(90));
    let a = archivist(conn, ArchivistConfig::default());

    let before = score_of(a.conn(), "stale");
    let report = a.report().unwrap();
    assert_eq!(report.memory_count, 1);
    assert_eq!(
        score_of(a.conn(), "stale"),
        before,
        "report() mutated a signal score; it is supposed to be a dry run"
    );
}

#[test]
fn an_empty_database_reports_zero_without_erroring() {
    let a = archivist(test_db(), ArchivistConfig::default());
    let report = a.report().unwrap();
    assert_eq!(report.memory_count, 0);
    assert!(report.duplicates.is_empty());
    assert!(report.reclassifications.is_empty());
    assert!(report.consolidation_candidates.is_empty());

    let run = a.run().unwrap();
    assert_eq!(run.decay_stats.decayed, 0);
    assert_eq!(run.decay_stats.boosted, 0);
}

// ── pluggable traits ───────────────────────────────────────────────

struct StubConsolidator;
impl Consolidator for StubConsolidator {
    fn consolidate(&self, a: &str, b: &str) -> anyhow::Result<Option<String>> {
        Ok(Some(format!("merged({a}|{b})")))
    }
}

struct StubIndexer;
impl Indexer for StubIndexer {
    fn generate_index(
        &self,
        wing: &str,
        memories: &[(String, String, Option<String>)],
    ) -> anyhow::Result<Option<String>> {
        Ok(Some(format!("{wing}:{}", memories.len())))
    }
}

/// `with_consolidator` / `with_indexer` must actually swap the implementation
/// out — the default is a no-op returning `None`, so a builder that dropped
/// the value would leave a consumer's LLM client silently unused.
#[test]
fn the_builder_methods_install_the_supplied_implementations() {
    let default = archivist(test_db(), ArchivistConfig::default());
    assert_eq!(
        default.consolidator().consolidate("a", "b").unwrap(),
        None,
        "the default consolidator should be the no-op"
    );
    assert_eq!(default.indexer().generate_index("w", &[]).unwrap(), None);

    let custom = archivist(test_db(), ArchivistConfig::default())
        .with_consolidator(Box::new(StubConsolidator))
        .with_indexer(Box::new(StubIndexer));

    assert_eq!(
        custom
            .consolidator()
            .consolidate("a", "b")
            .unwrap()
            .as_deref(),
        Some("merged(a|b)"),
        "with_consolidator did not install the supplied implementation"
    );
    assert_eq!(
        custom
            .indexer()
            .generate_index("ops", &[])
            .unwrap()
            .as_deref(),
        Some("ops:0"),
        "with_indexer did not install the supplied implementation"
    );
}

/// Opening from a real file must work and see the rows already there — the
/// path every consumer actually uses, as opposed to `from_conn`.
#[test]
fn opening_from_a_file_path_sees_existing_rows() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db = tmp.path().join("memory.db");
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY, key TEXT NOT NULL UNIQUE, content TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT 'core', wing TEXT, hall TEXT,
                signal_score REAL DEFAULT 0.5, visibility TEXT NOT NULL DEFAULT 'private',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                source TEXT, device_id BLOB, confidence REAL NOT NULL DEFAULT 1.0,
                last_reinforced_at TEXT
             );
             CREATE TABLE memory_spectrogram (
                memory_id TEXT PRIMARY KEY, entity_density REAL, action_type TEXT,
                decision_polarity REAL, causal_depth REAL, emotional_valence REAL,
                temporal_specificity REAL, novelty REAL, peak_dimensions TEXT,
                created_at TEXT DEFAULT (datetime('now'))
             );",
        )
        .unwrap();
        insert(&conn, "1", "a", "some content", "w", "fact");
    }

    let a = Archivist::open(&db).unwrap();
    assert_eq!(a.report().unwrap().memory_count, 1);
}

#[test]
fn opening_a_nonexistent_directory_is_an_error() {
    let path = std::path::Path::new("/nonexistent-dir-for-tests/memory.db");
    assert!(
        Archivist::open(path).is_err(),
        "opening a database under a nonexistent directory should fail"
    );
}
