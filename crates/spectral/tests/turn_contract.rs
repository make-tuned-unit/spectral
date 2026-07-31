//! Invariant tests for the agent-turn contract (`spectral::turn`).
//!
//! These are the feedback invariants from the turn-contract preregistration
//! (`docs/internal/turn-contract-prereg-2026-07-30.md`). They are the $0 gate:
//! every one must hold before any paid end-to-end run is considered.
//!
//! The load-bearing property is the asymmetry — retrieval alone changes
//! nothing, and only memories reported `Used` are ever strengthened.

use spectral::{
    Brain, MemoryOutcome, RecognitionContext, TurnPolicyVersion, TurnRequest, Visibility,
};
use spectral_graph::cascade_layers::CascadePipelineConfig;
use tempfile::TempDir;

fn seeded_brain(tmp: &TempDir) -> Brain {
    let brain = Brain::open(tmp.path()).unwrap();
    brain
        .remember(
            "deploy-runbook",
            "the staging deploy runbook lists the rollback steps",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "deploy-incident",
            "the staging deploy incident on Tuesday needed a rollback",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "deploy-owner",
            "the staging deploy owner rotates every sprint",
            Visibility::Private,
        )
        .unwrap();
    brain
}

/// Read the current signal score for a key, using the read-only turn path so
/// that observing the system does not perturb it.
fn score_of(brain: &Brain, key: &str) -> Option<f64> {
    let request = TurnRequest::query("staging deploy rollback", Visibility::Private);
    brain
        .turn(&request)
        .unwrap()
        .hits
        .iter()
        .find(|h| h.key == key)
        .map(|h| h.signal_score)
}

/// A turn that is never committed must leave memory state completely
/// unchanged, no matter how many times it runs. This is what auto-reinforce
/// at retrieval time cannot offer.
#[test]
fn uncommitted_turns_do_not_change_signal_scores() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);

    let before: Vec<(String, f64)> = {
        let r = brain
            .turn(&TurnRequest::query(
                "staging deploy rollback",
                Visibility::Private,
            ))
            .unwrap();
        r.hits
            .iter()
            .map(|h| (h.key.clone(), h.signal_score))
            .collect()
    };
    assert!(!before.is_empty(), "fixture must retrieve something");

    for _ in 0..5 {
        brain
            .turn(&TurnRequest::query(
                "staging deploy rollback",
                Visibility::Private,
            ))
            .unwrap();
    }

    let after: Vec<(String, f64)> = {
        let r = brain
            .turn(&TurnRequest::query(
                "staging deploy rollback",
                Visibility::Private,
            ))
            .unwrap();
        r.hits
            .iter()
            .map(|h| (h.key.clone(), h.signal_score))
            .collect()
    };

    assert_eq!(
        before, after,
        "repeated uncommitted turns must not change ranking or signal scores"
    );
}

/// Only `Used` earns reinforcement. `Wrong` and `Ignored` must leave the
/// memory exactly as it was — this is the correctness core of the contract.
#[test]
fn only_used_outcomes_reinforce() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);

    let result = brain
        .turn(&TurnRequest::query(
            "staging deploy rollback",
            Visibility::Private,
        ))
        .unwrap();
    assert!(
        result.receipt.delivered.len() >= 3,
        "fixture must deliver all three memories, got {}",
        result.receipt.delivered.len()
    );

    let before_used = score_of(&brain, "deploy-runbook").unwrap();
    let before_wrong = score_of(&brain, "deploy-incident").unwrap();
    let before_ignored = score_of(&brain, "deploy-owner").unwrap();

    let receipt = brain
        .record_turn_outcome(
            &result.receipt,
            &[
                ("deploy-runbook", MemoryOutcome::Used),
                ("deploy-incident", MemoryOutcome::Wrong),
                ("deploy-owner", MemoryOutcome::Ignored),
            ],
        )
        .unwrap();

    assert_eq!(receipt.reinforced, vec!["deploy-runbook".to_string()]);
    assert_eq!(
        receipt.not_reinforced,
        vec!["deploy-incident".to_string(), "deploy-owner".to_string()]
    );

    assert!(
        score_of(&brain, "deploy-runbook").unwrap() > before_used,
        "a Used memory must be reinforced"
    );
    assert_eq!(
        score_of(&brain, "deploy-incident").unwrap(),
        before_wrong,
        "a Wrong memory must never be strengthened"
    );
    assert_eq!(
        score_of(&brain, "deploy-owner").unwrap(),
        before_ignored,
        "an Ignored memory must never be strengthened"
    );
}

/// An outcome naming a memory the turn did not deliver is unattributable, and
/// accepting it would reopen the unearned-credit path this contract closes.
#[test]
fn outcome_for_undelivered_key_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    brain
        .remember(
            "unrelated-note",
            "the office plant watering schedule",
            Visibility::Private,
        )
        .unwrap();

    let result = brain
        .turn(&TurnRequest::query(
            "staging deploy rollback",
            Visibility::Private,
        ))
        .unwrap();

    let err = brain
        .record_turn_outcome(&result.receipt, &[("unrelated-note", MemoryOutcome::Used)])
        .unwrap_err();
    assert!(
        err.to_string().contains("did not deliver"),
        "expected an attribution error, got: {err}"
    );
}

/// Recognition runs over `observations` and never implicitly over the query.
/// Feeding questions to a content re-encounter engine is the documented cause
/// of the 0.9% real-query wing precision.
#[test]
fn recognition_runs_only_over_observations() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);

    // Query-only turn: retrieval happens, recognition does not.
    let query_only = brain
        .turn(&TurnRequest::query(
            "staging deploy rollback",
            Visibility::Private,
        ))
        .unwrap();
    assert!(
        !query_only.hits.is_empty(),
        "a query-only turn must still retrieve"
    );
    assert!(
        query_only.recognition.is_empty(),
        "a query must never be fed to recognition"
    );

    // Observation turn: one verdict per stimulus, positionally aligned, and
    // byte-identical to the standalone `recognize` path.
    let observations = [
        "the staging deploy runbook lists the rollback steps",
        "an entirely novel sentence about kayaking in fog",
    ];
    let request = TurnRequest {
        query: None,
        observations: &observations,
        visibility: Visibility::Private,
        context: RecognitionContext::empty(),
        policy: TurnPolicyVersion::V1,
    };
    let observed = brain.turn(&request).unwrap();

    assert!(observed.hits.is_empty(), "no query means no retrieval");
    assert_eq!(
        observed.recognition.len(),
        observations.len(),
        "one verdict per observation, positionally aligned"
    );
    for (i, stimulus) in observations.iter().enumerate() {
        let standalone = brain.recognize(stimulus).unwrap();
        assert_eq!(
            observed.recognition[i].verdict, standalone.verdict,
            "turn recognition must match standalone recognize for {stimulus:?}"
        );
    }
}

/// The turn path must deliver the same hits, in the same order, as the legacy
/// cascade path — only the feedback timing changes. This is the facade-parity
/// gate: if it fails, the migration is not behavior-preserving.
#[test]
fn turn_delivers_same_hits_as_legacy_cascade_path() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    let query = "staging deploy rollback";

    // Turn first — it writes nothing, so it cannot perturb the comparison.
    let turned = brain
        .turn(&TurnRequest::query(query, Visibility::Private))
        .unwrap();

    let legacy = brain
        .recall_cascade_scoped(
            query,
            &RecognitionContext::empty(),
            &CascadePipelineConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    let turned_ids: Vec<&str> = turned.hits.iter().map(|h| h.id.as_str()).collect();
    let legacy_ids: Vec<&str> = legacy.merged_hits.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(
        turned_ids, legacy_ids,
        "turn must deliver the legacy hit set in the legacy order"
    );
}

/// Two identical deliveries are two distinct real-world turns. The delivery
/// digest must match (so repeat deliveries stay detectable) while the
/// occurrence ids must differ (so exposure is not undercounted).
///
/// An earlier version of this contract conflated the two and asserted the ids
/// were equal. That was wrong: collapsing repeat deliveries into one identity
/// makes "delivered repeatedly and never used" uncountable.
#[test]
fn identical_deliveries_share_a_digest_but_are_distinct_occurrences() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);

    let a = brain
        .turn(&TurnRequest::query(
            "staging deploy rollback",
            Visibility::Private,
        ))
        .unwrap();
    let b = brain
        .turn(&TurnRequest::query(
            "staging deploy rollback",
            Visibility::Private,
        ))
        .unwrap();

    assert_eq!(
        a.receipt.delivery_digest, b.receipt.delivery_digest,
        "identical deliveries must share a delivery digest"
    );
    assert_ne!(
        a.receipt.id, b.receipt.id,
        "each turn occurrence must have its own identity"
    );
    assert_eq!(a.receipt.policy.as_str(), "v1");
}

/// Legacy `recall_*` behavior must be untouched: the default config still
/// writes back, so existing consumers see exactly today's semantics.
#[test]
fn legacy_recall_still_writes_back_by_default() {
    assert!(
        CascadePipelineConfig::default().write_back,
        "default write_back must stay true so recall_* semantics are unchanged"
    );
}
