//! `CentroidTracker` — the session-level ambient recognizer.
//!
//! It is exported from the crate root but had no test caller outside a bench
//! binary, leaving `stream.rs` the lowest-covered shipped file in the
//! recognition crate. It is a *stateful* lock machine (acquire / transfer /
//! silent-continue) whose scoring encodes two decisions the source documents
//! as measured, not arbitrary:
//!
//! - **rarity weighting** — "a topic every routine touches identifies none of
//!   them";
//! - **the size tax** — without it, "pure containment made 'general' segments
//!   attractors for every live session — measured 0/38 on specific-wing cues".
//!
//! Both are asserted below, so a refactor that dropped either would fail here
//! rather than silently degrade ambient recognition.

use spectral_recognition::{centroid_of, Centroid, CentroidConfig, CentroidTracker, Cue, Segment};
use std::collections::HashSet;

/// Build a cue directly: [wing, dow, hour_band, peak1..peak4, len_bucket].
/// Constructed by hand rather than via `make_cue` so a test controls the peak
/// buckets exactly instead of depending on a hash.
fn cue(dow: u16, hour: u16, peaks: [u16; 4]) -> Cue {
    Cue([0, dow, hour, peaks[0], peaks[1], peaks[2], peaks[3], 1])
}

fn centroid(id: &str, dow: u16, hour: u16, peaks: &[u16]) -> Centroid {
    Centroid {
        segment_id: id.into(),
        wing: "w".into(),
        dow,
        hour,
        peaks: peaks.iter().copied().collect::<HashSet<u16>>(),
    }
}

fn lock_id(t: &CentroidTracker) -> Option<String> {
    t.current_lock().map(|c| c.segment_id.clone())
}

// ── the lock lifecycle ─────────────────────────────────────────────

/// Below `min_cues` the tracker must stay silent regardless of how well the
/// first cue matches — identity is supposed to emerge from a sequence.
#[test]
fn no_lock_before_min_cues_however_good_the_match() {
    let mut t = CentroidTracker::new(CentroidConfig::default());
    t.enroll(centroid("morning", 1, 3, &[10, 11, 12, 13]));

    let events = t.observe(&cue(1, 3, [10, 11, 12, 13]), false);
    assert!(
        events.is_empty(),
        "locked on the first cue, before min_cues: {events:?}"
    );
    assert!(lock_id(&t).is_none());
}

/// With no enrolled centroids there is nothing to lock onto, and observing
/// must not panic on the empty catalog.
#[test]
fn an_empty_catalog_never_locks() {
    let mut t = CentroidTracker::new(CentroidConfig::default());
    for _ in 0..5 {
        assert!(t.observe(&cue(1, 3, [10, 11, 12, 13]), false).is_empty());
    }
    assert!(lock_id(&t).is_none());
}

/// A clear, unambiguous match acquires a lock once `min_cues` is reached, and
/// the event reports the segment it locked onto.
#[test]
fn a_clear_match_acquires_a_lock_and_reports_it() {
    let mut t = CentroidTracker::new(CentroidConfig::default());
    t.enroll(centroid("morning", 1, 3, &[10, 11, 12, 13]));
    t.enroll(centroid("evening", 5, 7, &[90, 91, 92, 93]));

    t.observe(&cue(1, 3, [10, 11, 0, 0]), false);
    let events = t.observe(&cue(1, 3, [12, 13, 0, 0]), false);

    assert!(
        events
            .iter()
            .any(|e| matches!(e, spectral_recognition::StreamEvent::LockAcquired { segment_id, .. } if segment_id == "morning")),
        "expected LockAcquired on 'morning', got {events:?}"
    );
    assert_eq!(lock_id(&t).as_deref(), Some("morning"));
}

/// Re-alert suppression is structural: a continuing match must fire NOTHING.
/// This is the property that keeps an ambient recognizer from spamming.
#[test]
fn a_continuing_lock_is_silent() {
    let mut t = CentroidTracker::new(CentroidConfig::default());
    t.enroll(centroid("morning", 1, 3, &[10, 11, 12, 13]));
    t.enroll(centroid("evening", 5, 7, &[90, 91, 92, 93]));

    t.observe(&cue(1, 3, [10, 11, 0, 0]), false);
    let acquired = t.observe(&cue(1, 3, [12, 13, 0, 0]), false);
    assert!(!acquired.is_empty(), "precondition: a lock was acquired");

    for _ in 0..4 {
        let events = t.observe(&cue(1, 3, [10, 12, 0, 0]), false);
        assert!(
            events.is_empty(),
            "a continuing lock re-alerted instead of staying silent: {events:?}"
        );
    }
    assert_eq!(lock_id(&t).as_deref(), Some("morning"));
}

/// A boundary ends the session: the running segment resets and the lock
/// releases **silently**, because "a session ending naturally is not a
/// divergence event".
#[test]
fn a_boundary_releases_the_lock_without_an_event() {
    let mut t = CentroidTracker::new(CentroidConfig::default());
    t.enroll(centroid("morning", 1, 3, &[10, 11, 12, 13]));
    t.enroll(centroid("evening", 5, 7, &[90, 91, 92, 93]));

    t.observe(&cue(1, 3, [10, 11, 0, 0]), false);
    t.observe(&cue(1, 3, [12, 13, 0, 0]), false);
    assert!(lock_id(&t).is_some(), "precondition: locked");

    let events = t.observe(&cue(5, 7, [90, 91, 0, 0]), true);
    assert!(
        events.is_empty(),
        "a boundary emitted an event; it should release silently: {events:?}"
    );
    assert!(
        lock_id(&t).is_none(),
        "the lock survived a session boundary"
    );
}

/// Switching routines mid-session transfers the lock and names both ends, so
/// a consumer can say "you moved from X to Y".
#[test]
fn switching_routines_transfers_the_lock() {
    let mut t = CentroidTracker::new(CentroidConfig::default());
    t.enroll(centroid("morning", 1, 3, &[10, 11, 12, 13]));
    t.enroll(centroid("evening", 5, 7, &[90, 91, 92, 93]));

    t.observe(&cue(1, 3, [10, 11, 0, 0]), false);
    t.observe(&cue(1, 3, [12, 13, 0, 0]), false);
    assert_eq!(lock_id(&t).as_deref(), Some("morning"));

    // No boundary — the session continues but the evidence swings.
    let mut transferred = None;
    for _ in 0..6 {
        for e in t.observe(&cue(5, 7, [90, 91, 92, 93]), false) {
            if let spectral_recognition::StreamEvent::LockTransferred { from, to, .. } = e {
                transferred = Some((from, to));
            }
        }
    }

    let (from, to) = transferred.expect("expected a LockTransferred event");
    assert_eq!(from, "morning");
    assert_eq!(to, "evening");
    assert_eq!(lock_id(&t).as_deref(), Some("evening"));
}

// ── the two documented scoring decisions ───────────────────────────

/// **Rarity weighting.** A topic present in every enrolled routine carries no
/// identifying evidence.
///
/// The discriminating setup matters: one centroid contains ONLY the ubiquitous
/// topic, so it pays almost no size tax and would otherwise be the runaway
/// winner. With IDF that topic weighs `ln(n/df) = ln(4/4) = 0`, the live
/// segment's total weight is 0, and the topic score collapses to 0 for
/// everyone — nothing locks. Without IDF the bare centroid locks immediately.
///
/// An earlier version of this test enrolled four similar centroids and
/// asserted "no lock"; it passed with rarity weighting REMOVED, because they
/// all tied and the margin rule blocked the lock anyway — so it could not
/// fail. Verified by mutation this time.
#[test]
fn a_topic_shared_by_every_routine_identifies_none_of_them() {
    let mut t = CentroidTracker::new(CentroidConfig::default());
    // Peak 42 is in all four. "bare" holds nothing else, so under pure
    // containment it scores 1.0 with a negligible size penalty.
    t.enroll(centroid("bare", 1, 3, &[42]));
    // The others are BROAD, so their size tax is heavy and "bare" wins by a
    // wide margin under pure containment. Without that spread the margin rule
    // blocks the lock on its own and the test cannot detect the mutation.
    for (i, id) in ["b", "c", "d"].iter().enumerate() {
        let base = 100 + i as u16 * 100;
        let mut peaks = vec![42];
        peaks.extend(base..base + 20);
        t.enroll(centroid(id, 1, 3, &peaks));
    }

    let mut events = Vec::new();
    for _ in 0..5 {
        events.extend(t.observe(&cue(1, 3, [42, 0, 0, 0]), false));
    }

    assert!(
        lock_id(&t).is_none(),
        "locked on a topic every routine shares — rarity weighting is not \
         being applied (got {:?}, events {events:?})",
        lock_id(&t)
    );
}

/// **The size tax.** A huge catch-all centroid must not win by containing
/// everything — the source records this as measured 0/38 on specific-wing
/// cues before the tax was added.
///
/// The discriminating setup: the live topics are a strict SUBSET of the
/// catch-all but only partly covered by the specific centroid. Under pure
/// containment the catch-all scores 3/3 against the specific one's 2/3 and
/// wins outright; the size tax charges it for its large uncovered mass so it
/// cannot.
///
/// An earlier version made the live set a subset of BOTH, which tied under
/// pure containment and was blocked by the margin rule — so it passed with the
/// tax removed and could not fail. Verified by mutation this time.
#[test]
fn a_catch_all_centroid_does_not_capture_a_specific_session() {
    let mut t = CentroidTracker::new(CentroidConfig::default());
    let everything: Vec<u16> = (1..=60).collect();
    t.enroll(centroid("general", 1, 3, &everything));
    t.enroll(centroid("specific", 1, 3, &[7, 8]));

    // 50 is inside "general" but not "specific".
    for _ in 0..5 {
        t.observe(&cue(1, 3, [7, 8, 50, 0]), false);
    }

    assert_ne!(
        lock_id(&t).as_deref(),
        Some("general"),
        "the catch-all centroid captured a specific session — the size tax is \
         not being applied"
    );
}

/// The `lock_margin` requires the leader to beat the runner-up by a factor.
/// Two identical centroids can never satisfy it, so an ambiguous session must
/// stay unlocked rather than arbitrarily picking one.
#[test]
fn two_indistinguishable_routines_never_lock() {
    let mut t = CentroidTracker::new(CentroidConfig::default());
    t.enroll(centroid("twin-a", 1, 3, &[10, 11, 12, 13]));
    t.enroll(centroid("twin-b", 1, 3, &[10, 11, 12, 13]));

    for _ in 0..6 {
        t.observe(&cue(1, 3, [10, 11, 12, 13]), false);
    }
    assert!(
        lock_id(&t).is_none(),
        "locked onto one of two indistinguishable routines; the margin rule \
         should have kept it unlocked"
    );
}

// ── centroid_of ────────────────────────────────────────────────────

/// `centroid_of` summarises a segment: modal day and hour band, and the union
/// of its non-zero topic peaks.
#[test]
fn centroid_of_takes_the_modal_rhythm_and_the_union_of_peaks() {
    let segment = Segment {
        id: "seg".into(),
        wing: "work".into(),
        cues: vec![
            cue(2, 4, [10, 11, 0, 0]),
            cue(2, 4, [12, 0, 0, 0]),
            cue(2, 5, [13, 0, 0, 0]),
            // A single outlier day must not win the mode.
            cue(6, 4, [10, 0, 0, 0]),
        ],
    };

    let c = centroid_of(&segment);
    assert_eq!(c.segment_id, "seg");
    assert_eq!(c.wing, "work");
    assert_eq!(c.dow, 2, "the modal day should win over a single outlier");
    assert_eq!(c.hour, 4, "the modal hour band should win");

    let mut peaks: Vec<u16> = c.peaks.into_iter().collect();
    peaks.sort_unstable();
    assert_eq!(
        peaks,
        vec![10, 11, 12, 13],
        "peaks should be the union of non-zero slots, with 0 excluded"
    );
}

/// An empty segment produces no peaks. Its `dow`/`hour` are whatever the
/// argmax over an all-zero histogram yields — `max_by_key` returns the LAST
/// maximum on ties, so they come out at the top of each range (7 and 8), not
/// 0. Pinned rather than "fixed": it is deterministic, no caller feeds empty
/// segments, and changing it would move a published bucket. The same
/// last-wins rule breaks ties in non-empty segments too.
#[test]
fn centroid_of_an_empty_segment_has_no_peaks_and_a_tie_broken_rhythm() {
    let c = centroid_of(&Segment {
        id: "empty".into(),
        wing: "w".into(),
        cues: vec![],
    });
    assert!(
        c.peaks.is_empty(),
        "an empty segment should carry no topics"
    );
    assert_eq!(
        c.dow, 7,
        "argmax over an all-zero histogram takes the last index"
    );
    assert_eq!(c.hour, 8);
}
