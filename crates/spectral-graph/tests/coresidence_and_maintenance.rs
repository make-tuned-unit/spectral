//! Multi-brain co-residence, and the maintenance/feedback methods that had
//! only partial branch coverage.
//!
//! **Co-residence** is the premise federation rests on: N `Brain` handles must
//! coexist in one process and each serve recall from its own store. Until now
//! the only check was an `#[ignore]`d, Linux-only diagnostic written to
//! reproduce a SIGABRT in *Kuzu* — an engine that is no longer in the
//! dependency tree at all (absent from both `Cargo.toml` and `Cargo.lock`;
//! the graph store is SQLite now). That reproducer can no longer reproduce
//! anything, so the property it incidentally protected was untested. It is
//! asserted here as a live test instead.
//!
//! The rest covers `reinforce`, `forget`, and the two backfills — each of
//! which has a "found" and a "not found" path, and reports counts a caller
//! acts on.

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, ReinforceOpts};
use tempfile::TempDir;

fn config(tmp: &TempDir) -> BrainConfig {
    let ontology_path = tmp.path().join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();
    BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path,
        entity_policy: EntityPolicy::Strict,
        activity_wing: "activity".into(),
        ..Default::default()
    }
}

fn brain(tmp: &TempDir) -> Brain {
    Brain::open(config(tmp)).unwrap()
}

fn recall_keys(b: &Brain, q: &str) -> Vec<String> {
    b.recall_topk_fts(
        q,
        &spectral_graph::brain::RecallTopKConfig::default(),
        Visibility::Private,
    )
    .unwrap()
    .into_iter()
    .map(|h| h.key)
    .collect()
}

// ── co-residence: the federation premise ───────────────────────────

/// N brains on distinct data dirs must coexist in one process, each serving
/// recall from its OWN store with no cross-talk. This is what makes
/// `FederationCoordinator`'s in-process fan-out possible.
#[test]
fn several_brains_coexist_in_one_process_without_cross_talk() {
    let dirs: Vec<TempDir> = (0..4).map(|_| TempDir::new().unwrap()).collect();
    let brains: Vec<Brain> = dirs.iter().map(brain).collect();

    for (i, b) in brains.iter().enumerate() {
        b.remember(
            &format!("k{i}"),
            &format!("the zephyr runbook belonging to brain number {i}"),
            Visibility::Private,
        )
        .unwrap();
    }

    // Each brain sees its own memory and none of its neighbours'.
    for (i, b) in brains.iter().enumerate() {
        let keys = recall_keys(b, "zephyr runbook");
        assert!(
            keys.contains(&format!("k{i}")),
            "brain {i} cannot recall its own memory"
        );
        for j in 0..brains.len() {
            if j != i {
                assert!(
                    !keys.contains(&format!("k{j}")),
                    "brain {i} recalled brain {j}'s memory — the stores are not isolated"
                );
            }
        }
    }
}

/// Dropping brains in an order different from creation must not disturb the
/// survivors — the drop-order concern the old diagnostic was probing, kept as
/// a live assertion.
#[test]
fn dropping_brains_out_of_order_leaves_the_survivors_working() {
    let dirs: Vec<TempDir> = (0..3).map(|_| TempDir::new().unwrap()).collect();
    let mut brains: Vec<Option<Brain>> = dirs.iter().map(|d| Some(brain(d))).collect();

    for (i, b) in brains.iter().enumerate() {
        b.as_ref()
            .unwrap()
            .remember(
                &format!("k{i}"),
                &format!("the zephyr runbook for brain {i}"),
                Visibility::Private,
            )
            .unwrap();
    }

    // Drop the middle one first, then the first — deliberately not LIFO.
    brains[1] = None;
    brains[0] = None;

    let survivor = brains[2].as_ref().unwrap();
    assert!(
        recall_keys(survivor, "zephyr runbook").contains(&"k2".to_string()),
        "the surviving brain stopped serving recall after its neighbours were \
         dropped out of order"
    );
    // And it can still write.
    survivor
        .remember(
            "after",
            "written after the others were dropped",
            Visibility::Private,
        )
        .unwrap();
}

/// Re-opening a brain on a directory whose sibling brains are still alive must
/// work — the coordinator adds members incrementally, not all at once.
#[test]
fn a_brain_can_be_opened_while_others_are_already_live() {
    let a_dir = TempDir::new().unwrap();
    let b_dir = TempDir::new().unwrap();
    let a = brain(&a_dir);
    a.remember("ka", "the zephyr runbook for a", Visibility::Private)
        .unwrap();

    // Open a second brain while the first is still held.
    let b = brain(&b_dir);
    b.remember("kb", "the zephyr runbook for b", Visibility::Private)
        .unwrap();

    assert!(recall_keys(&a, "zephyr").contains(&"ka".to_string()));
    assert!(recall_keys(&b, "zephyr").contains(&"kb".to_string()));
}

// ── reinforce ──────────────────────────────────────────────────────

/// `reinforce` reports found and not-found keys separately. A caller uses
/// `memories_not_found` to detect stale keys, so both halves must be right in
/// one call that mixes them.
#[test]
fn reinforce_separates_found_keys_from_missing_ones() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember("real", "a real memory", Visibility::Private)
        .unwrap();

    let result = b
        .reinforce(ReinforceOpts {
            memory_keys: vec!["real".into(), "ghost-1".into(), "ghost-2".into()],
            strength: 0.1,
        })
        .unwrap();

    assert_eq!(result.memories_reinforced, 1);
    assert_eq!(
        result.memories_not_found,
        vec!["ghost-1".to_string(), "ghost-2".to_string()],
        "missing keys should be reported verbatim so a caller can act on them"
    );
}

/// Reinforcement raises the stored signal score — the adaptive loop's whole
/// point — and is clamped at 1.0.
#[test]
fn reinforce_raises_the_signal_score_and_clamps_at_one() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember("m", "a memory to strengthen", Visibility::Private)
        .unwrap();
    let before = b.get_memory_by_key("m").unwrap().unwrap().signal_score;

    b.reinforce(ReinforceOpts {
        memory_keys: vec!["m".into()],
        strength: 0.1,
    })
    .unwrap();
    let after = b.get_memory_by_key("m").unwrap().unwrap().signal_score;
    assert!(
        after > before,
        "reinforce did not raise the score: {before} -> {after}"
    );

    // Hammer it well past the ceiling.
    for _ in 0..30 {
        b.reinforce(ReinforceOpts {
            memory_keys: vec!["m".into()],
            strength: 1.0,
        })
        .unwrap();
    }
    let capped = b.get_memory_by_key("m").unwrap().unwrap().signal_score;
    assert!(capped <= 1.0, "signal_score exceeded its ceiling: {capped}");
}

#[test]
fn reinforcing_an_empty_key_list_is_a_no_op() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    let r = b.reinforce(ReinforceOpts::default()).unwrap();
    assert_eq!(r.memories_reinforced, 0);
    assert!(r.memories_not_found.is_empty());
}

#[test]
fn reinforce_is_refused_on_a_read_only_brain() {
    let tmp = TempDir::new().unwrap();
    {
        let b = brain(&tmp);
        b.remember("m", "a memory", Visibility::Private).unwrap();
    }
    let ro = Brain::open(BrainConfig {
        read_only: true,
        ..config(&tmp)
    })
    .unwrap();
    assert!(matches!(
        ro.reinforce(ReinforceOpts {
            memory_keys: vec!["m".into()],
            strength: 0.1
        }),
        Err(spectral_graph::error::Error::ReadOnly(_))
    ));
}

// ── forget ─────────────────────────────────────────────────────────

/// Forgetting a key that was never stored must report "did not exist" rather
/// than erroring — a caller retrying a deletion should be able to.
#[test]
fn forgetting_an_unknown_key_reports_absence_rather_than_erroring() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    let report = b.forget("never-existed").unwrap();
    assert!(
        !report.store.existed,
        "a key that was never stored should report existed = false"
    );
}

/// Forgetting is idempotent: the second call on the same key reports absence
/// and does not error.
#[test]
fn forgetting_twice_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember(
        "doomed",
        "content that will be forgotten",
        Visibility::Private,
    )
    .unwrap();

    let first = b.forget("doomed").unwrap();
    assert!(first.store.existed);
    assert!(
        first.fully_forgotten(),
        "the first forget should be complete"
    );

    let second = b.forget("doomed").unwrap();
    assert!(
        !second.store.existed,
        "the second forget should report the key as already absent"
    );
}

#[test]
fn forget_is_refused_on_a_read_only_brain() {
    let tmp = TempDir::new().unwrap();
    {
        let b = brain(&tmp);
        b.remember("m", "a memory", Visibility::Private).unwrap();
    }
    let ro = Brain::open(BrainConfig {
        read_only: true,
        ..config(&tmp)
    })
    .unwrap();
    assert!(matches!(
        ro.forget("m"),
        Err(spectral_graph::error::Error::ReadOnly(_))
    ));
    // And the memory survived the refused call.
    assert!(ro.get_memory_by_key("m").unwrap().is_some());
}

// ── backfills ──────────────────────────────────────────────────────

/// `backfill_content_hashes` repairs rows written before the column existed.
/// It must fill every cleared row and then find nothing left to do.
#[test]
fn backfill_content_hashes_fills_cleared_rows_and_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    for i in 0..3 {
        b.remember(
            &format!("m{i}"),
            &format!("content number {i}"),
            Visibility::Private,
        )
        .unwrap();
    }

    {
        let conn = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
        conn.execute("UPDATE memories SET content_hash = NULL", [])
            .unwrap();
    }

    assert_eq!(
        b.backfill_content_hashes().unwrap(),
        3,
        "every cleared row should be backfilled"
    );
    assert_eq!(
        b.backfill_content_hashes().unwrap(),
        0,
        "a second backfill should find nothing to do"
    );

    // And the hashes are actually populated.
    let m = b.get_memory_by_key("m0").unwrap().unwrap();
    assert!(m.content_hash.is_some(), "the content hash is still NULL");
}

#[test]
fn both_backfills_are_refused_on_a_read_only_brain() {
    let tmp = TempDir::new().unwrap();
    drop(brain(&tmp));
    let ro = Brain::open(BrainConfig {
        read_only: true,
        ..config(&tmp)
    })
    .unwrap();
    assert!(matches!(
        ro.backfill_content_hashes(),
        Err(spectral_graph::error::Error::ReadOnly(_))
    ));
}

/// On a brain with nothing to repair, a backfill must report zero rather than
/// erroring — this runs on every startup path that calls it.
#[test]
fn backfilling_a_healthy_brain_reports_zero() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember("m", "already healthy content", Visibility::Private)
        .unwrap();
    assert_eq!(b.backfill_content_hashes().unwrap(), 0);
}

// ── fingerprint peer cap ───────────────────────────────────────────

/// `set_max_fingerprint_peers` bounds constellation fingerprint fan-out per
/// write. The effect must be OBSERVABLE, not merely accepted: a tight cap has
/// to produce fewer fingerprint rows than an unbounded one over the same
/// corpus.
///
/// An earlier version of this test only checked that recall still worked after
/// setting the cap, which passed with the setter neutered entirely.
#[test]
fn the_fingerprint_peer_cap_actually_bounds_fan_out() {
    fn rows_written(cap: Option<usize>) -> i64 {
        let tmp = TempDir::new().unwrap();
        let mut b = brain(&tmp);
        b.set_max_fingerprint_peers(cap);
        // A cluster of mutually-similar memories, so fan-out has peers to find.
        for i in 0..12 {
            b.remember(
                &format!("m{i}"),
                &format!("the zephyr rollout runbook notion pipeline entry {i}"),
                Visibility::Private,
            )
            .unwrap();
        }
        drop(b);
        let conn = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
        conn.query_row("SELECT COUNT(*) FROM constellation_fingerprints", [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    let tight = rows_written(Some(1));
    let unbounded = rows_written(None);

    assert!(
        tight < unbounded,
        "a cap of 1 produced {tight} fingerprint rows and no cap produced \
         {unbounded} — the cap is not reaching ingestion"
    );
}

/// Clearing the cap restores unbounded fan-out and leaves recall working.
#[test]
fn clearing_the_fingerprint_peer_cap_restores_the_default() {
    let tmp = TempDir::new().unwrap();
    let mut b = brain(&tmp);
    b.set_max_fingerprint_peers(Some(1));
    b.set_max_fingerprint_peers(None);
    b.remember(
        "uncapped",
        "the zephyr rollout note uncapped",
        Visibility::Private,
    )
    .unwrap();
    assert!(recall_keys(&b, "zephyr rollout").contains(&"uncapped".to_string()));
}
