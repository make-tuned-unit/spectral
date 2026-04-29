use std::path::PathBuf;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, ReinforceOpts};
use tempfile::TempDir;

fn open_brain(tmp: &TempDir) -> Brain {
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
    })
    .unwrap()
}

#[test]
fn reinforce_increases_signal_score() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    brain
        .remember(
            "test-key",
            "Decided to use Polybot for the weather prediction strategy",
            Visibility::Private,
        )
        .unwrap();

    let before = brain
        .recall("polybot weather prediction strategy", Visibility::Private)
        .unwrap();
    assert!(!before.memory_hits.is_empty());
    let score_before = before.memory_hits[0].signal_score;

    brain
        .reinforce(ReinforceOpts {
            memory_keys: vec!["test-key".into()],
            strength: 0.1,
        })
        .unwrap();

    let after = brain
        .recall("polybot weather prediction strategy", Visibility::Private)
        .unwrap();
    assert!(!after.memory_hits.is_empty());
    // Signal score should be higher (original + 0.1, minus negligible decay)
    assert!(
        after.memory_hits[0].signal_score > score_before,
        "expected signal to increase: before={score_before}, after={}",
        after.memory_hits[0].signal_score
    );
}

#[test]
fn reinforce_clamps_at_one() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    brain
        .remember(
            "clamp-key",
            "Decided to use Polybot for weather clamp",
            Visibility::Private,
        )
        .unwrap();

    // Reinforce 15 times with strength 0.1
    for _ in 0..15 {
        brain
            .reinforce(ReinforceOpts {
                memory_keys: vec!["clamp-key".into()],
                strength: 0.1,
            })
            .unwrap();
    }

    // Read signal_score directly from SQLite to verify clamping
    let result = brain
        .recall("polybot weather clamp", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());
    // Decayed score can be at most 1.0 (and decay on a just-reinforced memory is ~0)
    assert!(
        result.memory_hits[0].signal_score <= 1.0,
        "signal_score should be clamped to 1.0, got {}",
        result.memory_hits[0].signal_score
    );
}

#[test]
fn reinforce_updates_timestamp() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    brain
        .remember(
            "timestamp-key",
            "Decided to use Polybot for weather timestamp",
            Visibility::Private,
        )
        .unwrap();

    brain
        .reinforce(ReinforceOpts {
            memory_keys: vec!["timestamp-key".into()],
            strength: 0.1,
        })
        .unwrap();

    let result = brain
        .recall("polybot weather timestamp", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());
    assert!(
        result.memory_hits[0].last_reinforced_at.is_some(),
        "last_reinforced_at should be set after reinforcement"
    );
}

#[test]
fn reinforce_returns_not_found_for_missing_keys() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    let result = brain
        .reinforce(ReinforceOpts {
            memory_keys: vec!["nonexistent-key".into()],
            strength: 0.1,
        })
        .unwrap();

    assert_eq!(result.memories_reinforced, 0);
    assert_eq!(result.memories_not_found.len(), 1);
    assert_eq!(result.memories_not_found[0], "nonexistent-key");
}

#[test]
fn reinforce_invalidates_wing_cache() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    brain
        .remember(
            "cache-key",
            "Decided to use Polybot for weather cache",
            Visibility::Private,
        )
        .unwrap();

    // Populate wing cache by recalling
    let r1 = brain
        .recall("polybot weather cache", Visibility::Private)
        .unwrap();
    assert!(!r1.memory_hits.is_empty());
    let score1 = r1.memory_hits[0].signal_score;

    // Reinforce
    brain
        .reinforce(ReinforceOpts {
            memory_keys: vec!["cache-key".into()],
            strength: 0.2,
        })
        .unwrap();

    // Recall again — cache should be invalidated, showing updated score
    let r2 = brain
        .recall("polybot weather cache", Visibility::Private)
        .unwrap();
    assert!(!r2.memory_hits.is_empty());
    assert!(
        r2.memory_hits[0].signal_score > score1,
        "expected higher score after reinforce+cache invalidation"
    );
}

#[test]
fn decay_does_not_affect_recent_memories() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    let r = brain
        .remember(
            "recent-key",
            "Decided to use Polybot for weather recent",
            Visibility::Private,
        )
        .unwrap();

    let result = brain
        .recall("polybot weather recent", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());

    // A memory created just now should have no meaningful decay
    let decayed = result.memory_hits[0].signal_score;
    let raw = r.signal_score;
    assert!(
        (decayed - raw).abs() < 0.01,
        "recent memory should have negligible decay: raw={raw}, decayed={decayed}"
    );
}

#[test]
fn decay_applies_to_old_memories() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    brain
        .remember(
            "old-key",
            "Decided to use Polybot for weather old",
            Visibility::Private,
        )
        .unwrap();

    // Backdates created_at to 30 days ago via direct SQL
    {
        let db_path = tmp.path().join("memory.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE memories SET created_at = datetime('now', '-30 days') WHERE key = 'old-key'",
            [],
        )
        .unwrap();
    }

    // Re-open brain to clear any caches
    drop(brain);
    let brain = open_brain(&tmp);

    let result = brain
        .recall("polybot weather old", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());

    // 30 days = ~4.3 weeks. Decay = 4.3 * 0.01 = ~4.3%. Factor ~0.957.
    let hit = &result.memory_hits[0];
    assert!(
        hit.signal_score < 0.97 * 0.85, // raw score is ~0.85 for this content
        "old memory should have decayed: score={}",
        hit.signal_score
    );

    // Verify decay is READ-ONLY: the stored signal_score in SQLite must be unchanged.
    let stored_score: f64 = {
        let db_path = tmp.path().join("memory.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.query_row(
            "SELECT signal_score FROM memories WHERE key = 'old-key'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert!(
        (stored_score - 0.85).abs() < 0.02,
        "stored signal_score should be unchanged after recall, got {stored_score}"
    );
}

#[test]
fn decay_capped_at_50_percent() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    brain
        .remember(
            "ancient-key",
            "Decided to use Polybot for weather ancient",
            Visibility::Private,
        )
        .unwrap();

    // Backdate to 5 years ago
    {
        let db_path = tmp.path().join("memory.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE memories SET created_at = datetime('now', '-1825 days') WHERE key = 'ancient-key'",
            [],
        )
        .unwrap();
    }

    drop(brain);
    let brain = open_brain(&tmp);

    let result = brain
        .recall("polybot weather ancient", Visibility::Private)
        .unwrap();
    assert!(!result.memory_hits.is_empty());

    // Decay should be capped at 50%, so score >= raw * 0.5
    let hit = &result.memory_hits[0];
    assert!(
        hit.signal_score >= 0.85 * 0.49, // slightly below 0.5 for float tolerance
        "ancient memory decay should be capped at 50%: score={}",
        hit.signal_score
    );
    assert!(
        hit.signal_score <= 0.85 * 0.51,
        "ancient memory should be near 50% floor: score={}",
        hit.signal_score
    );
}

#[test]
fn recent_reinforcement_resets_decay_clock() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    brain
        .remember(
            "reset-key",
            "Decided to use Polybot for weather reset",
            Visibility::Private,
        )
        .unwrap();

    // Backdate to 30 days ago
    {
        let db_path = tmp.path().join("memory.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE memories SET created_at = datetime('now', '-30 days') WHERE key = 'reset-key'",
            [],
        )
        .unwrap();
    }

    drop(brain);
    let brain = open_brain(&tmp);

    // Before reinforcement — should show decay
    let before = brain
        .recall("polybot weather reset", Visibility::Private)
        .unwrap();
    assert!(!before.memory_hits.is_empty());
    let decayed_score = before.memory_hits[0].signal_score;

    // Reinforce — resets the decay clock to now
    brain
        .reinforce(ReinforceOpts {
            memory_keys: vec!["reset-key".into()],
            strength: 0.0, // zero strength so we only reset the clock
        })
        .unwrap();

    let after = brain
        .recall("polybot weather reset", Visibility::Private)
        .unwrap();
    assert!(!after.memory_hits.is_empty());

    // After reinforcement, the decay clock is reset so the score should be higher
    assert!(
        after.memory_hits[0].signal_score > decayed_score,
        "reinforcement should reset decay clock: before={decayed_score}, after={}",
        after.memory_hits[0].signal_score
    );
}

#[test]
fn reinforce_round_trip() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    // Write two memories
    brain
        .remember(
            "important",
            "Decided to use Polybot for weather prediction important decision strategy",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "trivial",
            "Polybot had a minor weather prediction config change details",
            Visibility::Private,
        )
        .unwrap();

    // Recall and reinforce only the important one
    let r1 = brain
        .recall("polybot weather prediction strategy", Visibility::Private)
        .unwrap();
    assert!(r1.memory_hits.len() >= 2);

    brain
        .reinforce(ReinforceOpts {
            memory_keys: vec!["important".into()],
            strength: 0.2,
        })
        .unwrap();

    // Recall again — the reinforced memory should rank higher
    let r2 = brain
        .recall("polybot weather prediction strategy", Visibility::Private)
        .unwrap();
    assert!(!r2.memory_hits.is_empty());

    let important_hit = r2.memory_hits.iter().find(|h| h.key == "important");
    let trivial_hit = r2.memory_hits.iter().find(|h| h.key == "trivial");

    if let (Some(imp), Some(triv)) = (important_hit, trivial_hit) {
        assert!(
            imp.signal_score >= triv.signal_score,
            "reinforced memory should rank >= unreinforced: important={}, trivial={}",
            imp.signal_score,
            triv.signal_score
        );
    }
}
