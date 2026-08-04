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

// ── V2Shaped: the library executes its own published policy ─────────
//
// `spectral::policy` described a configuration the library never ran: only the
// benchmark harness called `cascade_profile`, so a published number could not
// be reproduced through the public API. These pin that V2 closes the gap and
// that V1 is untouched.

/// V1 must not classify. Whatever the query looks like, it gets the generic
/// cascade config — so every existing caller is byte-for-byte unaffected.
#[test]
fn v1_does_not_classify_and_matches_generic_cascade() {
    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    // A counting-shaped query, which V2 would route to a k=60 profile.
    let q = "how many staging deploys were rolled back in total";

    let v1 = brain
        .turn(&TurnRequest {
            query: Some(q),
            observations: &[],
            visibility: Visibility::Private,
            context: RecognitionContext::empty(),
            policy: TurnPolicyVersion::V1,
        })
        .unwrap();

    let generic = brain
        .recall_cascade_scoped(
            q,
            &RecognitionContext::empty(),
            &CascadePipelineConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    let a: Vec<&str> = v1.hits.iter().map(|h| h.id.as_str()).collect();
    let b: Vec<&str> = generic.merged_hits.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(a, b, "V1 must remain the generic cascade path");
}

/// V2 must deliver exactly what the shape's own profile produces — i.e. the
/// library now runs the policy it publishes.
#[test]
fn v2_executes_the_published_per_shape_profile() {
    use spectral::policy::QuestionShape;

    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    let q = "how many staging deploys were rolled back in total";

    let shape = QuestionShape::classify(q);
    assert_eq!(
        shape,
        QuestionShape::Counting,
        "fixture query must be Counting"
    );

    let v2 = brain
        .turn(&TurnRequest {
            query: Some(q),
            observations: &[],
            visibility: Visibility::Private,
            context: RecognitionContext::empty(),
            policy: TurnPolicyVersion::V2Shaped,
        })
        .unwrap();

    // Same profile the policy publishes, with write-back off (turn is read-only).
    let profile = CascadePipelineConfig {
        write_back: false,
        ..shape.cascade_profile()
    };
    let direct = brain
        .recall_cascade_scoped(
            q,
            &RecognitionContext::empty(),
            &profile,
            Visibility::Private,
        )
        .unwrap();

    let a: Vec<&str> = v2.hits.iter().map(|h| h.id.as_str()).collect();
    let b: Vec<&str> = direct.merged_hits.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(
        a, b,
        "V2 must deliver exactly the published per-shape profile's result"
    );
    assert_eq!(v2.receipt.policy.as_str(), "v2-shaped");
}

/// Temporal shapes route OFF cascade (cascade measured ~-15pp on temporal).
/// V2 must honour `retrieval_route`; V1 must not.
#[test]
fn v2_honours_the_temporal_route_and_v1_does_not() {
    use spectral::policy::{QuestionShape, RetrievalRoute};

    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    let q = "when did the staging deploy incident happen";

    assert_eq!(QuestionShape::classify(q), QuestionShape::Temporal);
    assert_eq!(
        QuestionShape::classify(q).retrieval_route(),
        RetrievalRoute::TopkFts
    );

    let v2 = brain
        .turn(&TurnRequest {
            query: Some(q),
            observations: &[],
            visibility: Visibility::Private,
            context: RecognitionContext::empty(),
            policy: TurnPolicyVersion::V2Shaped,
        })
        .unwrap();

    // k is floored at 40 to match the harness, which applies the same floor
    // deliberately (max_results.max(40)) so temporal evidence reaching the top
    // 40 only after re-ranking is not cut.
    let topk = brain
        .recall_topk_fts(
            q,
            &spectral_graph::brain::RecallTopKConfig {
                k: QuestionShape::classify(q).cascade_profile().k.max(40),
                ..Default::default()
            },
            Visibility::Private,
        )
        .unwrap();

    let a: Vec<&str> = v2.hits.iter().map(|h| h.id.as_str()).collect();
    let b: Vec<&str> = topk.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(a, b, "V2 must route Temporal to top-k FTS, not cascade");

    // V1 stays on cascade for the same query.
    let v1 = brain
        .turn(&TurnRequest {
            query: Some(q),
            observations: &[],
            visibility: Visibility::Private,
            context: RecognitionContext::empty(),
            policy: TurnPolicyVersion::V1,
        })
        .unwrap();
    let cascade = brain
        .recall_cascade_scoped(
            q,
            &RecognitionContext::empty(),
            &CascadePipelineConfig::default(),
            Visibility::Private,
        )
        .unwrap();
    let c: Vec<&str> = v1.hits.iter().map(|h| h.id.as_str()).collect();
    let d: Vec<&str> = cascade.merged_hits.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(c, d, "V1 must stay on the cascade route");
}

/// The default policy must remain V1, so adding V2 changes nothing for anyone
/// who does not opt in.
#[test]
fn default_policy_is_still_v1() {
    assert_eq!(TurnPolicyVersion::default(), TurnPolicyVersion::V1);
    assert_eq!(
        TurnRequest::query("x", Visibility::Private).policy.as_str(),
        "v1"
    );
}

/// The top-k route floors `k` at 40, matching the harness's deliberate
/// `max_results.max(40)`. Without the floor the library would agree with the
/// measured configuration only by coincidence at today's profile values — so a
/// profile change could silently break parity with the published numbers.
#[test]
fn topk_route_floors_k_at_the_harness_value() {
    use spectral::policy::QuestionShape;

    let tmp = TempDir::new().unwrap();
    let brain = seeded_brain(&tmp);
    let q = "when did the staging deploy incident happen";

    // Today the Temporal profile is exactly 40, so floor and profile coincide.
    // Pin that, so a profile change surfaces here rather than silently
    // diverging from the harness.
    assert_eq!(
        QuestionShape::classify(q).cascade_profile().k,
        40,
        "Temporal profile k changed — re-verify the top-k floor against \
         spectral-bench-accuracy/src/retrieval.rs before updating this test"
    );

    let v2 = brain
        .turn(&TurnRequest {
            query: Some(q),
            observations: &[],
            visibility: Visibility::Private,
            context: RecognitionContext::empty(),
            policy: TurnPolicyVersion::V2Shaped,
        })
        .unwrap();
    let floored = brain
        .recall_topk_fts(
            q,
            &spectral_graph::brain::RecallTopKConfig {
                k: 40,
                ..Default::default()
            },
            Visibility::Private,
        )
        .unwrap();

    let a: Vec<&str> = v2.hits.iter().map(|h| h.id.as_str()).collect();
    let b: Vec<&str> = floored.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(a, b, "top-k route must use the floored k");
}
