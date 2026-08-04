//! A consumer must be able to reproduce the published context format.
//!
//! Until `spectral::render` existed, the session-grouped, dated, role-tagged
//! format behind the published LongMemEval-S number lived only in
//! `spectral-bench-accuracy`. The library's own formatter emitted
//! `[wing/hall] key: content` with no date, no grouping, no role tags and no
//! filler suppression — so a consumer calling `recall_*` and injecting the
//! result got a materially different prompt from the benchmarked one, even
//! with byte-identical retrieval.
//!
//! These tests exercise the whole consumer path — `Brain::open` → `remember`
//! → `recall_topk_fts` → `render` — through the public API only.

use spectral::render::{self, RenderOptions};
use spectral::{Brain, RecallTopKConfig, Visibility};
use tempfile::TempDir;

/// Two sessions of a conversation, keyed the way the ingest path keys turns.
fn brain_with_two_sessions() -> (TempDir, Brain) {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();

    let turns = [
        (
            "s1:turn:0:user",
            "I switched my main laptop to the framework 13 for repairability",
        ),
        ("s1:turn:1:assistant", "Got it."),
        (
            "s1:turn:2:assistant",
            "Noted — the Framework 13 is now your main laptop, chosen for repairability.",
        ),
        (
            "s2:turn:0:user",
            "The framework laptop battery life has been disappointing so far",
        ),
    ];
    for (key, content) in turns {
        brain.remember(key, content, Visibility::Private).unwrap();
    }
    (tmp, brain)
}

#[test]
fn consumer_can_render_the_published_session_grouped_format() {
    let (_tmp, brain) = brain_with_two_sessions();

    let hits = brain
        .recall_topk_fts(
            "framework laptop",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();
    assert!(!hits.is_empty(), "retrieval returned nothing to render");

    let lines = render::session_grouped(&hits, &RenderOptions::published());

    // Session headers are present and carry a date.
    let headers: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("--- Session"))
        .collect();
    assert!(!headers.is_empty(), "no session headers in {lines:#?}");
    for h in &headers {
        assert!(h.contains('('), "header carries no date: {h}");
    }

    // Turns are role-tagged.
    assert!(
        lines.iter().any(|l| l.starts_with("[user]")),
        "no role-tagged user turn in {lines:#?}"
    );

    // Short assistant filler is suppressed; the substantive assistant turn is kept.
    assert!(
        !lines.iter().any(|l| l.contains("Got it.")),
        "phatic assistant turn survived: {lines:#?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("chosen for repairability")),
        "substantive assistant turn was dropped: {lines:#?}"
    );
}

#[test]
fn rendering_is_deterministic_across_repeated_calls() {
    let (_tmp, brain) = brain_with_two_sessions();
    let hits = brain
        .recall_topk_fts(
            "framework laptop",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    let first = render::session_grouped(&hits, &RenderOptions::published());
    for _ in 0..5 {
        assert_eq!(
            render::session_grouped(&hits, &RenderOptions::published()),
            first
        );
    }
}

#[test]
fn relative_offsets_are_opt_in_and_change_only_the_date_tag() {
    let (_tmp, brain) = brain_with_two_sessions();
    let hits = brain
        .recall_topk_fts(
            "framework laptop",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    let plain = render::session_grouped(&hits, &RenderOptions::published());
    let dated = render::session_grouped(
        &hits,
        &RenderOptions::published()
            .with_question_date("2030/01/01 (Tue) 09:00")
            .with_relative_offsets(),
    );

    assert_eq!(plain.len(), dated.len(), "offsets changed the line count");
    for (p, d) in plain.iter().zip(&dated) {
        if p.starts_with("--- Session") {
            assert!(d.contains("ago"), "header gained no offset: {d}");
        } else {
            assert_eq!(p, d, "a non-header line changed");
        }
    }
}

#[test]
fn library_render_does_not_read_the_environment() {
    // The harness owns the env levers; the library must be a pure function of
    // its options. If this regresses, a consumer's output silently depends on
    // variables they never set.
    let (_tmp, brain) = brain_with_two_sessions();
    let hits = brain
        .recall_topk_fts(
            "framework laptop",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();

    let before = render::session_grouped(&hits, &RenderOptions::published());
    std::env::set_var("SPECTRAL_DATED_CONTEXT", "1");
    std::env::set_var("SPECTRAL_ACTOR_DESCRIPTIONS", "1");
    std::env::set_var("SPECTRAL_ASSISTANT_CAP_FRAC", "0.1");
    let after = render::session_grouped(&hits, &RenderOptions::published());
    std::env::remove_var("SPECTRAL_DATED_CONTEXT");
    std::env::remove_var("SPECTRAL_ACTOR_DESCRIPTIONS");
    std::env::remove_var("SPECTRAL_ASSISTANT_CAP_FRAC");

    assert_eq!(before, after, "library rendering read an env var");
}
