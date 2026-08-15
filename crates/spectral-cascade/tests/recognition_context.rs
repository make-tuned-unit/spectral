//! `RecognitionContext` — the ambient state carried into every recall.
//!
//! It is a six-setter builder feeding the re-ranking pipeline, and
//! `spectral-cascade` had two tests in the whole crate. The risk is the one
//! that has recurred across this codebase's builders: a setter whose value
//! never reaches the struct still compiles, still chains, and silently changes
//! what recall does.
//!
//! Two contracts here are subtler than the setters and are pinned explicitly:
//!
//! - **`now` defaults to the wall clock.** The field's own doc calls this
//!   "silently wrong for historical replay", which makes `with_now` the
//!   determinism seam — the same one the R20 anchor work is about.
//! - **`is_empty()` means "no *ambient* signal"**, and deliberately ignores
//!   `session_id` and `now`. Setting a session id must NOT make a context
//!   look populated, or layers keyed off `is_empty()` would start doing
//!   ambient work for a caller that supplied none.

use chrono::{DateTime, TimeZone, Utc};
use spectral_cascade::RecognitionContext;
use spectral_ingest::activity::ActivityEpisode;

fn episode(id: &str) -> ActivityEpisode {
    let at = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
    ActivityEpisode {
        id: id.into(),
        started_at: at,
        ended_at: at,
        bundle_id: "com.example.editor".into(),
        app_name: "Editor".into(),
        window_title: Some("a window".into()),
        url: None,
        excerpt: Some("some text".into()),
        source: "test".into(),
        source_event_count: 1,
        metadata: serde_json::Value::Null,
        wing: None,
    }
}

fn anchor() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2019, 3, 4, 10, 11, 12).unwrap()
}

// ── the empty baseline ─────────────────────────────────────────────

#[test]
fn an_empty_context_carries_no_ambient_signal() {
    let ctx = RecognitionContext::empty();
    assert!(ctx.is_empty());
    assert!(ctx.recent_activity.is_empty());
    assert!(ctx.focus_wing.is_none());
    assert!(ctx.persona.is_none());
    assert!(ctx.session_id.is_none());
}

/// `Default` is documented as `empty()`. Asserted behaviourally rather than by
/// reading the impl, so a `Default` that drifted would fail here.
#[test]
fn default_matches_empty() {
    let d = RecognitionContext::default();
    let e = RecognitionContext::empty();
    assert_eq!(d.is_empty(), e.is_empty());
    assert_eq!(d.recent_activity.len(), e.recent_activity.len());
    assert_eq!(d.focus_wing, e.focus_wing);
    assert_eq!(d.persona, e.persona);
    assert_eq!(d.session_id, e.session_id);
}

/// `empty()` anchors `now` to the wall clock — the behaviour the field's doc
/// warns is "silently wrong for historical replay". Pinned because it is the
/// reason `with_now` exists.
#[test]
fn an_empty_context_anchors_now_to_the_wall_clock() {
    let before = Utc::now();
    let ctx = RecognitionContext::empty();
    let after = Utc::now();
    assert!(
        ctx.now >= before && ctx.now <= after,
        "empty() should anchor `now` to the current wall clock, got {}",
        ctx.now
    );
}

// ── each setter reaches its field ──────────────────────────────────

#[test]
fn with_session_sets_only_the_session_id() {
    let ctx = RecognitionContext::empty().with_session("sess-1");
    assert_eq!(ctx.session_id.as_deref(), Some("sess-1"));
    assert!(ctx.focus_wing.is_none(), "with_session touched focus_wing");
    assert!(ctx.persona.is_none(), "with_session touched persona");
    assert!(ctx.recent_activity.is_empty());
}

#[test]
fn with_focus_wing_sets_only_the_focus_wing() {
    let ctx = RecognitionContext::empty().with_focus_wing("permagent");
    assert_eq!(ctx.focus_wing.as_deref(), Some("permagent"));
    assert!(ctx.session_id.is_none());
    assert!(ctx.persona.is_none());
}

#[test]
fn with_persona_sets_only_the_persona() {
    let ctx = RecognitionContext::empty().with_persona("reviewer");
    assert_eq!(ctx.persona.as_deref(), Some("reviewer"));
    assert!(ctx.focus_wing.is_none());
    assert!(ctx.session_id.is_none());
}

#[test]
fn with_recent_activity_sets_only_the_episodes() {
    let ctx = RecognitionContext::empty().with_recent_activity(vec![episode("e1"), episode("e2")]);
    assert_eq!(ctx.recent_activity.len(), 2);
    assert_eq!(ctx.recent_activity[0].id, "e1");
    assert!(ctx.focus_wing.is_none());
}

/// The determinism seam: `with_now` must override the wall-clock default, or
/// historical replay silently scores recency against today.
#[test]
fn with_now_overrides_the_wall_clock_default() {
    let ctx = RecognitionContext::empty().with_now(anchor());
    assert_eq!(
        ctx.now,
        anchor(),
        "with_now did not override the default anchor — historical replay \
         would score recency against the wall clock"
    );
}

// ── chaining ───────────────────────────────────────────────────────

/// Every setter chained together must all survive: a setter that rebuilt the
/// struct instead of mutating it would drop whatever came before.
#[test]
fn chaining_every_setter_preserves_all_of_them() {
    let ctx = RecognitionContext::empty()
        .with_session("sess-1")
        .with_now(anchor())
        .with_focus_wing("permagent")
        .with_persona("reviewer")
        .with_recent_activity(vec![episode("e1")]);

    assert_eq!(ctx.session_id.as_deref(), Some("sess-1"));
    assert_eq!(ctx.now, anchor());
    assert_eq!(ctx.focus_wing.as_deref(), Some("permagent"));
    assert_eq!(ctx.persona.as_deref(), Some("reviewer"));
    assert_eq!(ctx.recent_activity.len(), 1);
}

/// Chaining order must not matter — the same set of calls in a different
/// order yields the same context.
#[test]
fn chaining_order_does_not_change_the_result() {
    let forward = RecognitionContext::empty()
        .with_session("s")
        .with_focus_wing("w")
        .with_persona("p");
    let reverse = RecognitionContext::empty()
        .with_persona("p")
        .with_focus_wing("w")
        .with_session("s");

    assert_eq!(forward.session_id, reverse.session_id);
    assert_eq!(forward.focus_wing, reverse.focus_wing);
    assert_eq!(forward.persona, reverse.persona);
    assert_eq!(forward.is_empty(), reverse.is_empty());
}

/// Calling a setter twice keeps the LAST value, which is what a caller
/// overriding a default expects.
#[test]
fn setting_a_field_twice_keeps_the_last_value() {
    let ctx = RecognitionContext::empty()
        .with_focus_wing("first")
        .with_focus_wing("second");
    assert_eq!(ctx.focus_wing.as_deref(), Some("second"));

    let ctx = RecognitionContext::empty()
        .with_recent_activity(vec![episode("a")])
        .with_recent_activity(vec![episode("b"), episode("c")]);
    assert_eq!(
        ctx.recent_activity.len(),
        2,
        "the second call should replace, not append"
    );
    assert_eq!(ctx.recent_activity[0].id, "b");
}

// ── is_empty: ambient signal only ──────────────────────────────────

/// The subtle contract. `is_empty()` asks "is there ambient signal", so a
/// session id — which is bookkeeping, not signal — must leave it true.
/// Layers use this as a fast path to SKIP context-conditional work; if a
/// session id flipped it, they would start doing ambient work for a caller
/// who supplied none.
#[test]
fn a_session_id_alone_does_not_make_a_context_non_empty() {
    let ctx = RecognitionContext::empty().with_session("sess-1");
    assert!(
        ctx.is_empty(),
        "a session id is bookkeeping, not ambient signal — it must not make \
         the context look populated"
    );
}

/// Likewise a time anchor: `now` is always set, so it can never be the thing
/// that makes a context non-empty.
#[test]
fn a_time_anchor_alone_does_not_make_a_context_non_empty() {
    assert!(RecognitionContext::empty().with_now(anchor()).is_empty());
}

/// Each of the three ambient signals independently makes the context
/// non-empty — asserted one at a time so a missing clause in `is_empty()`
/// fails rather than being masked by the others.
#[test]
fn each_ambient_signal_independently_makes_the_context_non_empty() {
    assert!(
        !RecognitionContext::empty()
            .with_recent_activity(vec![episode("e1")])
            .is_empty(),
        "recent activity should count as ambient signal"
    );
    assert!(
        !RecognitionContext::empty()
            .with_focus_wing("permagent")
            .is_empty(),
        "a focus wing should count as ambient signal"
    );
    assert!(
        !RecognitionContext::empty()
            .with_persona("reviewer")
            .is_empty(),
        "a persona should count as ambient signal"
    );
}

/// An explicitly empty activity vector is still no signal — passing
/// `vec![]` must not flip the flag.
#[test]
fn an_empty_activity_vector_is_still_no_signal() {
    assert!(RecognitionContext::empty()
        .with_recent_activity(vec![])
        .is_empty());
}
