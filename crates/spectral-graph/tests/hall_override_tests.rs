//! R40 — an explicit hall on write, and `set_hall` on an existing memory.
//!
//! The hall routes TACT tier‑1 and is baked into every constellation
//! fingerprint hash the memory participates in, so both paths are checked
//! against the fingerprint table directly, not just the memories row.

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use spectral_ingest::{fingerprint::make_fingerprint_hash, TimeBucket};

fn test_brain() -> (Brain, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let ontology_path = dir.path().join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();
    let brain = Brain::open(BrainConfig {
        data_dir: dir.path().to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
        ..Default::default()
    })
    .unwrap();
    (brain, dir)
}

fn fingerprints(dir: &std::path::Path) -> Vec<(String, String, String, String, String, String)> {
    let conn = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT anchor_memory_id, target_memory_id, anchor_hall, target_hall, wing, time_delta_bucket, fingerprint_hash
             FROM constellation_fingerprints ORDER BY id",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            format!("{}|{}", r.get::<_, String>(5)?, r.get::<_, String>(6)?),
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// Content that matches NO default hall rule, so the classifier says `event`.
const NEUTRAL: &str =
    "Automation job nightly-report-sequencer completed in 33315ms with 0 messages";

#[test]
fn explicit_hall_on_write_bypasses_the_classifier() {
    let (brain, _dir) = test_brain();
    let auto = brain
        .remember_with(
            "k_auto",
            NEUTRAL,
            RememberOpts {
                visibility: Visibility::Private,
                wing: Some("ops".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        auto.hall.as_deref(),
        Some("event"),
        "precondition: no rule matches"
    );

    let explicit = brain
        .remember_with(
            "k_explicit",
            NEUTRAL,
            RememberOpts {
                visibility: Visibility::Private,
                wing: Some("ops".into()),
                hall: Some("fact".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(explicit.hall.as_deref(), Some("fact"));
    let m = brain.get_memory(&explicit.memory_id).unwrap().unwrap();
    assert_eq!(m.hall.as_deref(), Some("fact"));

    // Blank override falls back to the classifier rather than storing "".
    let blank = brain
        .remember_with(
            "k_blank",
            NEUTRAL,
            RememberOpts {
                visibility: Visibility::Private,
                wing: Some("ops".into()),
                hall: Some("   ".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(blank.hall.as_deref(), Some("event"));
}

#[test]
fn set_hall_rewrites_the_memory_and_rehashes_every_fingerprint_it_touches() {
    let (brain, dir) = test_brain();
    let opts = || RememberOpts {
        visibility: Visibility::Private,
        wing: Some("ops".into()),
        ..Default::default()
    };
    // Three memories in one wing: a is anchor for b and c; b is anchor for c.
    let a = brain.remember_with("k_a", NEUTRAL, opts()).unwrap();
    let _b = brain
        .remember_with("k_b", "Deploy of atlas finished with exit code 0", opts())
        .unwrap();
    let _c = brain
        .remember_with("k_c", "Backup of the vault archive verified", opts())
        .unwrap();
    let before = fingerprints(dir.path());
    assert!(
        before
            .iter()
            .any(|f| f.0 == a.memory_id || f.1 == a.memory_id),
        "precondition: memory a participates in at least one fingerprint"
    );
    for f in &before {
        assert_eq!(f.2, "event");
        assert_eq!(f.3, "event");
    }

    assert!(brain.set_hall(&a.memory_id, "fact").unwrap());
    let m = brain.get_memory(&a.memory_id).unwrap().unwrap();
    assert_eq!(m.hall.as_deref(), Some("fact"));

    let after = fingerprints(dir.path());
    assert_eq!(
        after.len(),
        before.len(),
        "set_hall must not add or drop fingerprints"
    );
    let mut touched = 0;
    for (anchor, target, ah, th, wing, bucket_hash) in &after {
        let (bucket, hash) = bucket_hash.split_once('|').unwrap();
        let expect_ah = if anchor == &a.memory_id {
            "fact"
        } else {
            "event"
        };
        let expect_th = if target == &a.memory_id {
            "fact"
        } else {
            "event"
        };
        assert_eq!(ah, expect_ah, "anchor_hall on {anchor}->{target}");
        assert_eq!(th, expect_th, "target_hall on {anchor}->{target}");
        // The stored hash must equal the canonical hash of the stored fields —
        // that is what fingerprint_search will look up by.
        let want = make_fingerprint_hash(ah, th, wing, TimeBucket::parse(bucket).unwrap());
        assert_eq!(hash, want, "hash of {anchor}->{target} is stale");
        if anchor == &a.memory_id || target == &a.memory_id {
            touched += 1;
        }
    }
    assert!(touched >= 1);

    // Unknown id: false, no error. Empty hall: error.
    assert!(!brain.set_hall("no-such-id", "fact").unwrap());
    assert!(brain.set_hall(&a.memory_id, "  ").is_err());
}

/// `set_wing` must move the memory AND re-pair it: fingerprints formed among
/// the old wing's peers are dropped, and new ones are generated against the
/// destination wing.
///
/// This is the property that distinguishes it from `set_hall`. A hall change
/// alters what a pair says; a wing change alters who the pair is with, so
/// re-hashing in place would leave the memory pointing at a wing it has left —
/// invisible to any test that only checks the `wing` column.
#[test]
fn set_wing_moves_the_memory_and_repairs_its_fingerprints() {
    let (brain, dir) = test_brain();
    let opts = |w: &str| RememberOpts {
        visibility: Visibility::Private,
        wing: Some(w.to_string()),
        ..Default::default()
    };
    // Two peers in `alpha`, two in `beta`, then move one alpha memory to beta.
    let a1 = brain.remember_with("k_a1", NEUTRAL, opts("alpha")).unwrap();
    brain
        .remember_with(
            "k_a2",
            "Deploy of atlas finished with exit code 0",
            opts("alpha"),
        )
        .unwrap();
    brain
        .remember_with("k_b1", "Backup of the vault archive verified", opts("beta"))
        .unwrap();
    brain
        .remember_with(
            "k_b2",
            "Ledger export wrote 219 rows to archive-west",
            opts("beta"),
        )
        .unwrap();

    let pairs_of = |id: &str| -> Vec<(String, String)> {
        let conn = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT wing, CASE WHEN anchor_memory_id = ?1 THEN target_memory_id
                                   ELSE anchor_memory_id END
                 FROM constellation_fingerprints
                 WHERE anchor_memory_id = ?1 OR target_memory_id = ?1",
            )
            .unwrap();
        let out = stmt
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        out
    };

    let before = pairs_of(&a1.memory_id);
    assert!(
        !before.is_empty() && before.iter().all(|(w, _)| w == "alpha"),
        "precondition: a1 is paired inside alpha, got {before:?}"
    );

    assert!(brain.set_wing(&a1.memory_id, "beta").unwrap());
    assert_eq!(
        brain
            .get_memory(&a1.memory_id)
            .unwrap()
            .unwrap()
            .wing
            .as_deref(),
        Some("beta")
    );

    let after = pairs_of(&a1.memory_id);
    assert!(
        after.iter().all(|(w, _)| w == "beta"),
        "every surviving pair must belong to the destination wing, got {after:?}"
    );
    assert!(
        !after.is_empty(),
        "the memory must be re-paired with its new wing's peers, not left orphaned"
    );

    // Unknown id is false, blank wing is an error — same contract as set_hall.
    assert!(!brain.set_wing("no-such-id", "beta").unwrap());
    assert!(brain.set_wing(&a1.memory_id, "  ").is_err());
}
