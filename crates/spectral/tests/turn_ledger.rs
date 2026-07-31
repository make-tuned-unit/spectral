//! Turn-outcome ledger — evidence-integrity gate.
//!
//! These are the six preregistered pass/fail rules for the durable ledger
//! (`docs/internal/turn-contract-prereg-2026-07-30.md`). They validate
//! **evidence integrity only** — that exposure, use, and rejection survive the
//! call, durably and exactly once. They do NOT validate any lifecycle
//! hypothesis: nothing here claims that outcome-conditioned consolidation or
//! decay improves memory quality. That requires real bipolar production
//! outcomes and a separate preregistration.

use spectral::{Brain, MemoryOutcome, TurnRequest, Visibility};
use tempfile::TempDir;

fn seeded_brain(tmp: &TempDir) -> Brain {
    let brain = Brain::open(tmp.path()).unwrap();
    for (k, c) in [
        (
            "led-1",
            "the staging deploy runbook lists the rollback steps",
        ),
        ("led-2", "the staging deploy incident needed a rollback"),
        ("led-3", "the staging deploy owner rotates every sprint"),
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

fn evidence_for(brain: &Brain, key: &str) -> Option<spectral_ingest::MemoryOutcomeEvidence> {
    brain
        .memory_outcome_evidence(100)
        .unwrap()
        .into_iter()
        .find(|e| e.memory_key == key)
}

/// Rule 1 — two byte-identical deliveries produce two distinct ledger
/// occurrences. Exposure must not be undercounted.
#[test]
fn identical_deliveries_are_two_ledger_occurrences() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);

    let a = deploy_turn(&brain);
    let b = deploy_turn(&brain);
    assert_eq!(a.receipt.delivery_digest, b.receipt.delivery_digest);
    assert_ne!(a.receipt.id, b.receipt.id);

    let ev = evidence_for(&brain, "led-1").expect("led-1 must be delivered");
    assert_eq!(
        ev.delivered, 2,
        "two deliveries must count as two exposures, got {}",
        ev.delivered
    );
    assert_eq!(ev.unreported, 2, "uncommitted turns are 'unreported'");
    assert_eq!(ev.used, 0);
}

/// Rule 2 — the ledger is durable: rank and outcome survive a close/reopen.
#[test]
fn ledger_survives_reopen_with_exact_rank_and_outcome() {
    let tmp = TempDir::new().unwrap();
    let (digest, ranks) = {
        let brain = seeded_brain(&tmp);
        let turn = deploy_turn(&brain);
        brain
            .record_turn_outcome(&turn.receipt, &[("led-1", MemoryOutcome::Used)])
            .unwrap();
        let ranks: Vec<(String, usize)> = turn
            .receipt
            .delivered
            .iter()
            .map(|d| (d.key.clone(), d.rank))
            .collect();
        (turn.receipt.delivery_digest.clone(), ranks)
    };
    assert!(!digest.is_empty());

    let brain = Brain::open(tmp.path()).unwrap();
    let ev = evidence_for(&brain, "led-1").expect("evidence must survive reopen");
    assert_eq!(ev.used, 1, "Used outcome must be durable");
    assert_eq!(ev.delivered, 1);
    let expected_rank = ranks.iter().find(|(k, _)| k == "led-1").unwrap().1 as u64;
    assert_eq!(
        ev.best_rank,
        Some(expected_rank),
        "delivered rank must be durable"
    );
}

/// Rule 3 — replaying an outcome commit changes neither counts nor scores.
/// Reinforcement is additive, so a non-idempotent replay would double-count.
#[test]
fn replaying_an_outcome_commit_is_a_no_op() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    let turn = deploy_turn(&brain);

    brain
        .record_turn_outcome(&turn.receipt, &[("led-1", MemoryOutcome::Used)])
        .unwrap();
    let after_first = evidence_for(&brain, "led-1").unwrap();
    let score_after_first = deploy_turn(&brain)
        .hits
        .iter()
        .find(|h| h.key == "led-1")
        .map(|h| h.signal_score)
        .unwrap();

    // Replay the identical commit.
    brain
        .record_turn_outcome(&turn.receipt, &[("led-1", MemoryOutcome::Used)])
        .unwrap();
    let after_replay = evidence_for(&brain, "led-1").unwrap();
    let score_after_replay = deploy_turn(&brain)
        .hits
        .iter()
        .find(|h| h.key == "led-1")
        .map(|h| h.signal_score)
        .unwrap();

    assert_eq!(
        after_first.used, after_replay.used,
        "replay must not increment the used count"
    );
    assert_eq!(
        score_after_first, score_after_replay,
        "replay must not reinforce a second time"
    );
}

/// Rule 4 — `Used` reinforces exactly once; `Wrong`, `Ignored` and
/// `Unreported` never reinforce.
#[test]
fn only_used_reinforces_and_all_outcomes_are_recorded() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    let turn = deploy_turn(&brain);
    assert!(turn.receipt.delivered.len() >= 3);

    brain
        .record_turn_outcome(
            &turn.receipt,
            &[
                ("led-1", MemoryOutcome::Used),
                ("led-2", MemoryOutcome::Wrong),
                // led-3 deliberately left unreported.
            ],
        )
        .unwrap();

    let used = evidence_for(&brain, "led-1").unwrap();
    let wrong = evidence_for(&brain, "led-2").unwrap();
    let unreported = evidence_for(&brain, "led-3").unwrap();

    assert_eq!((used.used, used.wrong, used.unreported), (1, 0, 0));
    assert_eq!((wrong.used, wrong.wrong, wrong.unreported), (0, 1, 0));
    assert_eq!(
        (unreported.used, unreported.wrong, unreported.unreported),
        (0, 0, 1),
        "a member with no reported outcome must persist as 'unreported', not vanish"
    );
}

/// Rule 5 — an outcome naming an undelivered key leaves the ledger and every
/// signal score untouched.
#[test]
fn rejected_outcome_leaves_ledger_and_scores_unchanged() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    brain
        .remember(
            "led-outsider",
            "unrelated plant watering",
            Visibility::Private,
        )
        .unwrap();
    let turn = deploy_turn(&brain);

    let before = brain.memory_outcome_evidence(100).unwrap();
    let err = brain
        .record_turn_outcome(&turn.receipt, &[("led-outsider", MemoryOutcome::Used)])
        .unwrap_err();
    assert!(err.to_string().contains("did not deliver"));

    let after = brain.memory_outcome_evidence(100).unwrap();
    assert_eq!(before, after, "a rejected commit must change nothing");
}

/// Rule 6 — `delivered_never_used` answers the question the flat retrieval
/// event log structurally cannot.
#[test]
fn delivered_never_used_is_answerable() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);

    for _ in 0..3 {
        let turn = deploy_turn(&brain);
        brain
            .record_turn_outcome(
                &turn.receipt,
                &[
                    ("led-1", MemoryOutcome::Used),
                    ("led-2", MemoryOutcome::Ignored),
                    ("led-3", MemoryOutcome::Ignored),
                ],
            )
            .unwrap();
    }

    let used = evidence_for(&brain, "led-1").unwrap();
    let never = evidence_for(&brain, "led-2").unwrap();

    assert!(!used.delivered_never_used(3), "led-1 was used every time");
    assert!(
        never.delivered_never_used(3),
        "led-2 was delivered {} times and used {} — must be flagged",
        never.delivered,
        never.used
    );
    assert_eq!(never.ignored, 3);
}

/// The ledger holds memory ids AND keys, so `forget` must erase its rows —
/// otherwise a forgotten memory leaves residue and the deletion guarantee is
/// broken at a substrate the D1 sweep would newly find.
#[test]
fn forget_erases_ledger_rows() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    let turn = deploy_turn(&brain);
    brain
        .record_turn_outcome(&turn.receipt, &[("led-1", MemoryOutcome::Used)])
        .unwrap();
    assert!(evidence_for(&brain, "led-1").is_some());

    brain.forget("led-1").unwrap();

    assert!(
        evidence_for(&brain, "led-1").is_none(),
        "forget must cascade to the turn ledger"
    );
}
