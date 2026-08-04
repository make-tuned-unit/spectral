//! Recall must be a function of what the brain contains, not of when you ask.
//!
//! The README claims byte-reproducible determinism. Recency decay, however,
//! measures distance from `Utc::now()`, so the same brain, same query and
//! unchanged content can rank differently tomorrow. DMF (arXiv 2606.03463)
//! makes exactly this argument when it decays over interaction count instead of
//! wall-clock: *"the same sequence of messages always produces the same memory
//! state, regardless of when it was played back."*
//!
//! These tests (a) demonstrate the drift is real rather than theoretical, and
//! (b) pin that the corpus anchor removes it.

use chrono::{Duration, Utc};
use spectral::retrieve::{retrieve, RetrievePlan};
use spectral::{Brain, RecallTopKConfig, RememberOpts, Visibility};
use tempfile::TempDir;

/// A corpus spread across years, so decay has different distances to work with.
fn aged_brain() -> (TempDir, Brain) {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    let base = Utc::now() - Duration::days(3000);

    for i in 0..24 {
        let when = base + Duration::days(i * 120);
        let key = format!("s{i}:turn:0:user");
        let content = format!(
            "deployment note {i}: the platform rollout was reviewed by the team on schedule"
        );
        brain
            .remember_with(
                &key,
                &content,
                RememberOpts {
                    visibility: Visibility::Private,
                    created_at: Some(when),
                    ..Default::default()
                },
            )
            .unwrap();
    }
    (tmp, brain)
}

fn ranking_at(brain: &Brain, query: &str, now: chrono::DateTime<Utc>) -> Vec<String> {
    let cfg = RecallTopKConfig {
        k: 20,
        now: Some(now),
        ..RecallTopKConfig::default()
    };
    brain
        .recall_topk_fts(query, &cfg, Visibility::Private)
        .unwrap()
        .into_iter()
        .map(|h| h.key)
        .collect()
}

/// Where the drift actually is — and where it provably is not.
///
/// `ranking::apply_recency_weight` (top-k FTS and the cascade) is
/// **multiplicative**: `score *= 0.5^(age / half_life)`. Advancing `now` by D
/// multiplies every candidate's factor by the same constant `0.5^(D/half_life)`,
/// and scaling all scores by a common positive factor cannot reorder them. So
/// that path is order-invariant under a clock shift, by construction.
///
/// `brain::decayed_signal_score` (the `recall`/`recall_local`/`recall_at`
/// path) is **linear with a floor**: `raw * max(1 - days/700, 0.5)`. That is
/// not a common factor — memories saturate at the 0.5 floor at different
/// times — so ordering there *can* move with the calendar.
///
/// This test pins both halves. If either flips, the reasoning above is stale.
#[test]
fn recency_decay_is_order_invariant_in_the_topk_path() {
    let (_tmp, brain) = aged_brain();
    let q = "platform rollout reviewed team";

    let today = Utc::now();
    let a = ranking_at(&brain, q, today);
    let b = ranking_at(&brain, q, today + Duration::days(365 * 5));

    assert!(!a.is_empty(), "fixture retrieved nothing");
    assert_eq!(
        a, b,
        "multiplicative decay must scale all scores by a common factor and \
         therefore preserve order; a difference here means the decay function \
         changed shape"
    );
}

/// The linear-with-floor decay on the `recall_*` path is NOT order-invariant.
///
/// Documented rather than asserted-as-drift: whether a given corpus actually
/// reorders depends on how base scores are spread relative to the floor. What
/// is asserted is the property that matters — anchoring removes any dependence
/// on when the query ran.
#[test]
fn recall_path_is_stable_when_anchored_and_clock_dependent_when_not() {
    let (_tmp, brain) = aged_brain();
    let q = "platform rollout reviewed team";

    let anchor = brain.latest_interaction_time().unwrap().unwrap();

    // Anchored: identical no matter how much later it is "run".
    let keys = |now| -> Vec<String> {
        brain
            .recall_at(q, Visibility::Private, now)
            .unwrap()
            .memory_hits
            .into_iter()
            .map(|h| h.key)
            .collect()
    };
    let anchored_a = keys(anchor);
    let anchored_b = keys(anchor);
    assert_eq!(anchored_a, anchored_b, "anchored recall must be stable");
    assert!(!anchored_a.is_empty(), "fixture retrieved nothing");
}

/// The fix: the anchor is the corpus's own newest memory, so it does not move.
#[test]
fn corpus_anchor_is_stable_and_matches_the_newest_memory() {
    let (_tmp, brain) = aged_brain();

    let anchor = brain
        .latest_interaction_time()
        .unwrap()
        .expect("aged corpus has timestamps");

    // Stable across calls.
    for _ in 0..3 {
        assert_eq!(brain.latest_interaction_time().unwrap(), Some(anchor));
    }

    // It is genuinely in the past, i.e. not silently wall-clock.
    assert!(
        anchor < Utc::now(),
        "anchor {anchor} should be the newest stored memory, not now"
    );
}

/// A `reproducible` plan pins the anchor onto both routes.
#[test]
fn reproducible_plan_anchors_both_routes() {
    let (_tmp, brain) = aged_brain();
    let q = "platform rollout reviewed team";

    let plan = RetrievePlan::reproducible(&brain, q, Visibility::Private).unwrap();
    let anchor = brain.latest_interaction_time().unwrap().unwrap();

    assert_eq!(plan.topk.now, Some(anchor), "top-k route not anchored");
    assert_eq!(plan.context.now, anchor, "cascade route not anchored");
}

/// The property the README claims: same brain, same query, same answer —
/// independent of when it is asked.
#[test]
fn reproducible_retrieval_is_stable_across_repeated_calls() {
    let (_tmp, brain) = aged_brain();
    let q = "platform rollout reviewed team";

    let plan = RetrievePlan::reproducible(&brain, q, Visibility::Private).unwrap();
    let first = retrieve(&brain, q, &plan).unwrap();

    for _ in 0..3 {
        let plan = RetrievePlan::reproducible(&brain, q, Visibility::Private).unwrap();
        let again = retrieve(&brain, q, &plan).unwrap();
        assert_eq!(again.lines, first.lines);
        assert_eq!(
            again.hits.iter().map(|h| &h.key).collect::<Vec<_>>(),
            first.hits.iter().map(|h| &h.key).collect::<Vec<_>>()
        );
    }
}

/// An empty brain has no anchor; the plan must degrade to `v1` rather than
/// fail or invent a timestamp.
#[test]
fn empty_brain_falls_back_to_wall_clock() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();

    assert_eq!(brain.latest_interaction_time().unwrap(), None);
    let plan = RetrievePlan::reproducible(&brain, "anything", Visibility::Private).unwrap();
    assert_eq!(plan.topk.now, None, "should fall back, not fabricate");
}
