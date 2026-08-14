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
    aged_brain_inserted_in(&(0..24).collect::<Vec<_>>())
}

/// The same corpus, inserted in a caller-chosen order.
///
/// Insertion order is rowid order is — absent a tiebreak — the order equally
/// scored FTS rows come back in, which becomes the `1.0 - i/n` base score the
/// re-ranker starts from. Which memory is at which base rank is therefore a
/// property of the fixture, and tests that only ever insert chronologically
/// cannot see it.
fn aged_brain_inserted_in(order: &[i64]) -> (TempDir, Brain) {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    let base = Utc::now() - Duration::days(3000);

    for &i in order {
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

/// Where the drift actually is (R16, 2026-08-07).
///
/// An earlier version of this test asserted that the top-k path is
/// order-invariant under a clock shift, reasoning that
/// `ranking::apply_recency_weight` is **multiplicative** (`score *= 0.5^(age /
/// half_life)`) and that a common positive factor cannot reorder. That
/// reasoning described a function the top-k path **no longer calls**.
/// `ranking::apply_reranking_pipeline` applies recency **additively**:
///
/// ```text
/// scores[i]  = 1.0 - i/n          // base: FTS rank position
/// scores[i] += 0.1 * 0.5^(age_days / half_life)   // freshness, ADDED
/// ```
///
/// Advancing `now` shrinks every freshness term toward zero while the base
/// term stays put, so any pair whose FTS-rank order opposes its age order
/// crosses over at some clock offset. The old test passed only because its
/// fixture inserted chronologically *and* the FTS `ORDER BY` carried no
/// tiebreak, so equally scored rows came back in rowid order — making rank
/// order and age order agree, under which no such pair exists. R16 replaced
/// that accident with `ORDER BY bm25(...), m.id`; rank order is now a stable
/// function of content, uncorrelated with age, and the clock dependence is
/// visible on any corpus rather than only on shuffled ones.
///
/// `brain::decayed_signal_score` (the `recall`/`recall_local`/`recall_at`
/// path) is **linear with a floor**: `raw * max(1 - days/700, 0.5)`. Not a
/// common factor either — memories saturate at the floor at different times.
///
/// Neither path is clock-invariant. The fix that ships is the **corpus
/// anchor**: pin `now` to the newest stored memory so the calendar stops being
/// an input at all. That is what the rest of this file tests.
///
/// The clock dependence itself is **open as R20** in
/// `docs/internal/REPAIR_REGISTER.md` — every candidate fix is a default-path
/// ranking change and needs its own prereg and oracle A/B. This test is the
/// re-baseline R20 permits under Rule 5: it asserts the drift with `assert_ne!`
/// rather than tolerating it, so whichever fix eventually lands turns this red
/// and forces R20 to be closed deliberately instead of drifting shut.
#[test]
fn topk_additive_recency_reorders_under_a_clock_shift() {
    let (_tmp, brain) = aged_brain();
    let q = "platform rollout reviewed team";

    let today = Utc::now();
    let a = ranking_at(&brain, q, today);
    let b = ranking_at(&brain, q, today + Duration::days(365 * 5));

    assert!(!a.is_empty(), "fixture retrieved nothing");
    assert_ne!(
        a, b,
        "additive freshness on a fixed rank base must be able to reorder as \
         the clock advances; equality here means recency went back to being a \
         common multiplicative factor, and the doc comment above is now stale"
    );

    // Same clock, same answer — the drift is the calendar, not nondeterminism.
    assert_eq!(
        a,
        ranking_at(&brain, q, today),
        "same anchor must be stable"
    );
}

/// What R16 actually buys: two brains holding the same memories rank them the
/// same way, no matter what order they were written in.
///
/// Before the `, m.id` tiebreak this failed — equally scored FTS rows came
/// back in rowid order, so a brain built by replaying a log backwards ranked
/// differently from one built forwards, with identical content. That is the
/// reproducibility claim in the README, and it was untrue for tied documents.
#[test]
fn topk_ranking_is_independent_of_insertion_order() {
    let q = "platform rollout reviewed team";
    // Same anchor for both, so the clock dependence above is held fixed and
    // insertion order is the only variable.
    let anchor = Utc::now();

    let (_t1, forwards) = aged_brain();
    // Fixed permutation, no RNG — reproduces exactly.
    let shuffled: Vec<i64> = vec![
        23, 5, 17, 2, 11, 20, 8, 14, 1, 22, 6, 18, 3, 12, 21, 9, 15, 0, 13, 7, 19, 4, 16, 10,
    ];
    let (_t2, jumbled) = aged_brain_inserted_in(&shuffled);

    let a = ranking_at(&forwards, q, anchor);
    let b = ranking_at(&jumbled, q, anchor);

    assert!(!a.is_empty(), "fixture retrieved nothing");
    assert_eq!(
        a, b,
        "identical content written in a different order must rank identically; \
         a difference means the bm25 ORDER BY lost its `, m.id` tiebreak and \
         the LIMIT boundary is being decided by the query plan again"
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

/// R20 seam: `anchor_to_corpus` pins the DEFAULT (`now: None`) top-k anchor
/// to the corpus's newest memory instead of the wall clock.
///
/// The fixture's newest memory is FIVE YEARS old, so the corpus anchor and
/// the wall clock sit on opposite sides of the drift the test above pins: at
/// wall clock every freshness term is ≈0 (ranking collapses toward FTS
/// order), at the corpus anchor the newest memory carries freshness 1.0
/// (+0.1 against rank gaps of 1/24). An inert flag therefore CANNOT pass —
/// the flag-on ranking must equal the explicit-corpus-anchor ranking and
/// differ from the flag-off default.
#[test]
fn anchor_to_corpus_makes_the_default_ranking_clock_free() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    let base = Utc::now() - Duration::days(365 * 5 + 3000);
    for i in 0..24i64 {
        brain
            .remember_with(
                &format!("s{i}:turn:0:user"),
                &format!(
                    "deployment note {i}: the platform rollout was reviewed by the team on schedule"
                ),
                RememberOpts {
                    visibility: Visibility::Private,
                    created_at: Some(base + Duration::days(i * 120)),
                    ..Default::default()
                },
            )
            .unwrap();
    }
    let q = "platform rollout reviewed team";

    let keys = |cfg: &RecallTopKConfig| -> Vec<String> {
        brain
            .recall_topk_fts(q, cfg, Visibility::Private)
            .unwrap()
            .into_iter()
            .map(|h| h.key)
            .collect()
    };

    let anchored = keys(&RecallTopKConfig {
        k: 20,
        anchor_to_corpus: true,
        ..RecallTopKConfig::default()
    });
    let wall_clock_default = keys(&RecallTopKConfig {
        k: 20,
        ..RecallTopKConfig::default()
    });
    let explicit_corpus = keys(&RecallTopKConfig {
        k: 20,
        now: Some(brain.latest_interaction_time().unwrap().expect("non-empty")),
        ..RecallTopKConfig::default()
    });

    assert_eq!(
        anchored, explicit_corpus,
        "the flag must route the default anchor to latest_interaction_time"
    );
    assert_ne!(
        anchored, wall_clock_default,
        "on a 5-years-stale corpus the corpus anchor and the wall clock must \
         disagree; equality means the flag is inert"
    );

    // Explicit `now` always wins over the flag.
    let explicit_wins = keys(&RecallTopKConfig {
        k: 20,
        now: Some(Utc::now()),
        anchor_to_corpus: true,
        ..RecallTopKConfig::default()
    });
    assert_eq!(explicit_wins, wall_clock_default);
}
