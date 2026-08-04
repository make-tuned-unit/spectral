//! Deferred turn-delivery write — the three preregistered correctness gates
//! (`docs/internal/deferred-delivery-prereg-2026-08-04.md`). These must pass
//! BEFORE the latency gate is measured: a fast mode that silently drops
//! outcomes is worse than the slow one.

use spectral::{Brain, MemoryOutcome, TurnRequest, Visibility};
use tempfile::TempDir;

fn seeded_brain(tmp: &TempDir, deferred: bool) -> Brain {
    let mut brain = Brain::open(tmp.path()).unwrap();
    brain.set_async_turn_delivery(deferred);
    for (k, c) in [
        (
            "def-1",
            "the staging deploy runbook lists the rollback steps",
        ),
        ("def-2", "the staging deploy incident needed a rollback"),
        ("def-3", "the staging deploy owner rotates every sprint"),
    ] {
        brain.remember(k, c, Visibility::Private).unwrap();
    }
    brain
}

fn deploy_turn(brain: &Brain) -> spectral::TurnResult {
    brain
        .turn(&TurnRequest::query(
            "staging deploy rollback",
            Visibility::Private,
        ))
        .unwrap()
}

fn evidence_for(brain: &Brain, key: &str) -> spectral_ingest::MemoryOutcomeEvidence {
    brain
        .memory_outcome_evidence(100)
        .unwrap()
        .into_iter()
        .find(|e| e.memory_key == key)
        .unwrap_or_else(|| panic!("{key} must have ledger evidence"))
}

/// Gate 1 — an outcome committed immediately after a deferred turn lands
/// completely. Without the per-occurrence ordering await, this commit races
/// the spawned delivery write, UPDATEs zero rows, and every outcome vanishes
/// while the delivery lands as all-'unreported'.
#[test]
fn deferred_outcome_commit_cannot_race_its_delivery() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp, true);

    let result = deploy_turn(&brain);
    assert!(
        result.receipt.delivered.len() >= 2,
        "need >=2 hits to exercise mixed outcomes"
    );
    let k0 = result.receipt.delivered[0].key.clone();
    let k1 = result.receipt.delivered[1].key.clone();

    // Commit IMMEDIATELY — maximum race pressure against the spawned write.
    brain
        .record_turn_outcome(
            &result.receipt,
            &[
                (k0.as_str(), MemoryOutcome::Used),
                (k1.as_str(), MemoryOutcome::Ignored),
            ],
        )
        .unwrap();

    let e0 = evidence_for(&brain, &k0);
    assert_eq!(
        (e0.used, e0.unreported),
        (1, 0),
        "Used must land, not vanish"
    );
    let e1 = evidence_for(&brain, &k1);
    assert_eq!(
        (e1.ignored, e1.unreported),
        (1, 0),
        "Ignored must land, not vanish"
    );
}

/// Gate 2 — with the mode off (the default), nothing changes: the delivery is
/// synchronous, evidence is immediately durable, and flush is a no-op.
#[test]
fn off_mode_is_synchronous_and_flush_is_noop() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp, false);

    let result = deploy_turn(&brain);
    let k0 = &result.receipt.delivered[0].key;
    // No flush, no outcome: exposure is already durable on the sync path.
    assert_eq!(evidence_for(&brain, k0).delivered, 1);
    brain.flush_turn_deliveries().unwrap();
    assert_eq!(evidence_for(&brain, k0).delivered, 1);
}

/// Gate 3 — flush drains every in-flight delivery, and the exposure rows
/// survive reopen (durability of the flushed state, proven on disk).
#[test]
fn flush_drains_all_pending_deliveries_durably() {
    let tmp = TempDir::new().unwrap();
    let key = {
        let brain = seeded_brain(&tmp, true);
        let mut key = String::new();
        for _ in 0..5 {
            key = deploy_turn(&brain).receipt.delivered[0].key.clone();
        }
        brain.flush_turn_deliveries().unwrap();
        assert_eq!(
            evidence_for(&brain, &key).delivered,
            5,
            "all five exposures visible after flush"
        );
        key
    };

    let reopened = Brain::open(tmp.path()).unwrap();
    let ev = evidence_for(&reopened, &key);
    assert_eq!(
        (ev.delivered, ev.unreported),
        (5, 5),
        "uncommitted exposures survive reopen as unreported"
    );
}
