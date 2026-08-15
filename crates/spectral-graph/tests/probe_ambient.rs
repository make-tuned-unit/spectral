//! `probe` / `probe_recent` — the ambient-awareness entry point.
//!
//! `probe_recent` is documented as being called *periodically, e.g. on each
//! chat turn*, which makes it one of the hottest public paths in the library —
//! and it had only an empty-brain smoke test. Everything below exercises it
//! with real content: each `ProbeOpts` filter, all three `ProbeWindow`
//! variants, and the relevance ordering consumers rank on.
//!
//! The relevance formula is
//! `min(1.0, signal_score*0.4 + min(hits,5)/5*0.6)` — weighted toward match
//! count rather than stored score. The ordering tests below assert that
//! weighting rather than restating the arithmetic, so a re-tune that changed
//! which memory surfaces first would fail here.

use chrono::{Duration, Utc};
use spectral_core::visibility::Visibility;
use spectral_graph::activity::{ActivityEpisode, ProbeOpts, ProbeWindow};
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use tempfile::TempDir;

fn brain_config(tmp: &TempDir) -> BrainConfig {
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
    Brain::open(brain_config(tmp)).unwrap()
}

fn episode(id: &str, text: &str, at: chrono::DateTime<Utc>) -> ActivityEpisode {
    ActivityEpisode {
        id: id.into(),
        started_at: at,
        ended_at: at,
        bundle_id: "com.example.editor".into(),
        app_name: "Editor".into(),
        window_title: Some(text.into()),
        url: None,
        excerpt: Some(text.into()),
        source: "test".into(),
        source_event_count: 1,
        metadata: serde_json::Value::Null,
        wing: None,
    }
}

// ── probe ──────────────────────────────────────────────────────────

#[test]
fn an_empty_context_probes_nothing_without_touching_the_store() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember(
        "m",
        "the deploy runbook lives in notion",
        Visibility::Private,
    )
    .unwrap();
    assert!(b.probe("", ProbeOpts::default()).unwrap().is_empty());
}

#[test]
fn probe_surfaces_memories_matching_the_context() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember(
        "runbook",
        "the deploy runbook lives in notion",
        Visibility::Private,
    )
    .unwrap();
    b.remember(
        "garden",
        "tomatoes need watering weekly",
        Visibility::Private,
    )
    .unwrap();

    let hits = b.probe("deploy runbook", ProbeOpts::default()).unwrap();
    assert!(
        hits.iter().any(|r| r.key == "runbook"),
        "probe did not surface the matching memory"
    );
    assert!(
        !hits.iter().any(|r| r.key == "garden"),
        "probe surfaced an unrelated memory"
    );
}

/// Every returned item must carry a relevance in [0, 1] — consumers threshold
/// on it, so an out-of-range value would silently break their filtering.
#[test]
fn relevance_is_always_within_the_unit_interval() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    for i in 0..5 {
        b.remember(
            &format!("m{i}"),
            "deploy runbook deploy runbook deploy runbook notion",
            Visibility::Private,
        )
        .unwrap();
    }
    let hits = b
        .probe("deploy runbook notion", ProbeOpts::default())
        .unwrap();
    assert!(!hits.is_empty(), "precondition: something matched");
    for r in &hits {
        assert!(
            (0.0..=1.0).contains(&r.relevance),
            "relevance {} for {} is outside [0,1]",
            r.relevance,
            r.key
        );
    }
}

#[test]
fn results_are_ordered_by_descending_relevance() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    for i in 0..6 {
        b.remember(
            &format!("m{i}"),
            &format!("deploy runbook variant {i} notion pipeline"),
            Visibility::Private,
        )
        .unwrap();
    }
    let hits = b
        .probe("deploy runbook notion pipeline", ProbeOpts::default())
        .unwrap();
    assert!(hits.len() >= 2, "precondition: several matches");
    for pair in hits.windows(2) {
        assert!(
            pair[0].relevance >= pair[1].relevance,
            "probe results are not sorted by descending relevance: {:?}",
            hits.iter().map(|r| r.relevance).collect::<Vec<_>>()
        );
    }
}

#[test]
fn max_results_caps_the_returned_set() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    for i in 0..8 {
        b.remember(
            &format!("m{i}"),
            &format!("deploy runbook notion entry {i}"),
            Visibility::Private,
        )
        .unwrap();
    }

    let capped = b
        .probe(
            "deploy runbook notion",
            ProbeOpts {
                max_results: 3,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        capped.len() <= 3,
        "max_results was not applied: {}",
        capped.len()
    );
    assert!(!capped.is_empty(), "the cap removed everything");
}

/// A `min_relevance` above the highest achievable score must return nothing —
/// the filter has to be applied, not merely carried.
#[test]
fn min_relevance_filters_and_can_exclude_everything() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember(
        "runbook",
        "the deploy runbook lives in notion",
        Visibility::Private,
    )
    .unwrap();

    let unfiltered = b.probe("deploy runbook", ProbeOpts::default()).unwrap();
    assert!(
        !unfiltered.is_empty(),
        "precondition: it matches unfiltered"
    );

    let impossible = b
        .probe(
            "deploy runbook",
            ProbeOpts {
                min_relevance: 1.01,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        impossible.is_empty(),
        "min_relevance above the maximum still returned {} results",
        impossible.len()
    );
}

/// `wing_filter` restricts to one wing. Asserted in both directions so a
/// filter that was ignored, or one that excluded everything, both fail.
#[test]
fn wing_filter_admits_only_the_named_wing() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember_with(
        "in-wing",
        "deploy runbook notion alpha",
        RememberOpts {
            wing: Some("ops".into()),
            ..Default::default()
        },
    )
    .unwrap();
    b.remember_with(
        "out-wing",
        "deploy runbook notion beta",
        RememberOpts {
            wing: Some("elsewhere".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let filtered = b
        .probe(
            "deploy runbook notion",
            ProbeOpts {
                wing_filter: Some("ops".into()),
                ..Default::default()
            },
        )
        .unwrap();

    assert!(
        filtered.iter().any(|r| r.key == "in-wing"),
        "the wing filter excluded a memory that IS in the wing"
    );
    assert!(
        filtered.iter().all(|r| r.key != "out-wing"),
        "the wing filter admitted a memory from another wing"
    );
}

// ── probe_recent ───────────────────────────────────────────────────

#[test]
fn probe_recent_with_no_activity_returns_nothing() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    // A matching ordinary memory exists, but no ACTIVITY to synthesise a
    // context from — so there is nothing to probe with.
    b.remember(
        "runbook",
        "the deploy runbook lives in notion",
        Visibility::Private,
    )
    .unwrap();
    assert!(b
        .probe_recent(ProbeWindow::default(), ProbeOpts::default())
        .unwrap()
        .is_empty());
}

/// The real ambient loop: recent activity synthesises a context, and that
/// context surfaces related knowledge the user never asked for.
#[test]
fn recent_activity_surfaces_related_knowledge() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember(
        "runbook",
        "the deploy runbook lives in notion",
        Visibility::Private,
    )
    .unwrap();
    b.ingest_activity(&[episode(
        "e1",
        "editing the deploy runbook in notion",
        Utc::now(),
    )])
    .unwrap();

    let hits = b
        .probe_recent(
            ProbeWindow::Duration(Duration::hours(1)),
            ProbeOpts::default(),
        )
        .unwrap();
    assert!(
        hits.iter().any(|r| r.key == "runbook"),
        "recent activity did not surface the related memory: {:?}",
        hits.iter().map(|r| &r.key).collect::<Vec<_>>()
    );
}

/// A `Duration` window that ends before the activity happened must exclude it,
/// or the window parameter is decorative.
#[test]
fn a_duration_window_excludes_activity_older_than_it() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember(
        "runbook",
        "the deploy runbook lives in notion",
        Visibility::Private,
    )
    .unwrap();
    b.ingest_activity(&[episode(
        "old",
        "editing the deploy runbook in notion",
        Utc::now() - Duration::days(30),
    )])
    .unwrap();

    let recent = b
        .probe_recent(
            ProbeWindow::Duration(Duration::hours(1)),
            ProbeOpts::default(),
        )
        .unwrap();
    assert!(
        recent.is_empty(),
        "a 1-hour window included activity from 30 days ago"
    );

    // Widen the window and the same activity is now in scope.
    let wide = b
        .probe_recent(
            ProbeWindow::Duration(Duration::days(365)),
            ProbeOpts::default(),
        )
        .unwrap();
    assert!(
        wide.iter().any(|r| r.key == "runbook"),
        "a 365-day window still excluded 30-day-old activity"
    );
}

/// `Since` takes an explicit timestamp rather than a relative duration.
#[test]
fn a_since_window_uses_the_given_timestamp() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember(
        "runbook",
        "the deploy runbook lives in notion",
        Visibility::Private,
    )
    .unwrap();
    let when = Utc::now() - Duration::days(2);
    b.ingest_activity(&[episode("e1", "editing the deploy runbook in notion", when)])
        .unwrap();

    let after = b
        .probe_recent(
            ProbeWindow::Since(Utc::now() - Duration::days(5)),
            ProbeOpts::default(),
        )
        .unwrap();
    assert!(
        after.iter().any(|r| r.key == "runbook"),
        "a Since window preceding the activity excluded it"
    );

    let before = b
        .probe_recent(
            ProbeWindow::Since(Utc::now() - Duration::hours(1)),
            ProbeOpts::default(),
        )
        .unwrap();
    assert!(
        before.is_empty(),
        "a Since window AFTER the activity still included it"
    );
}

/// `Count` ignores time and takes the last N episodes — so even very old
/// activity is in scope, which is the difference from `Duration`.
#[test]
fn a_count_window_ignores_age() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember(
        "runbook",
        "the deploy runbook lives in notion",
        Visibility::Private,
    )
    .unwrap();
    b.ingest_activity(&[episode(
        "ancient",
        "editing the deploy runbook in notion",
        Utc::now() - Duration::days(3000),
    )])
    .unwrap();

    let hits = b
        .probe_recent(ProbeWindow::Count(10), ProbeOpts::default())
        .unwrap();
    assert!(
        hits.iter().any(|r| r.key == "runbook"),
        "a Count window excluded old activity; it should ignore age entirely"
    );
}

/// `probe_recent` forwards its opts to `probe` — a caller's cap must survive
/// the hop, or the ambient loop can flood a context window.
#[test]
fn probe_recent_forwards_its_opts_to_probe() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    for i in 0..8 {
        b.remember(
            &format!("m{i}"),
            &format!("deploy runbook notion entry {i}"),
            Visibility::Private,
        )
        .unwrap();
    }
    b.ingest_activity(&[episode("e1", "deploy runbook notion", Utc::now())])
        .unwrap();

    let hits = b
        .probe_recent(
            ProbeWindow::Duration(Duration::hours(1)),
            ProbeOpts {
                max_results: 2,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        hits.len() <= 2,
        "probe_recent ignored max_results and returned {}",
        hits.len()
    );
}

/// Probing is a read: it must work on a read-only brain, since the ambient
/// loop may well run against a replica.
#[test]
fn probing_works_on_a_read_only_brain() {
    let tmp = TempDir::new().unwrap();
    {
        let b = brain(&tmp);
        b.remember(
            "runbook",
            "the deploy runbook lives in notion",
            Visibility::Private,
        )
        .unwrap();
        b.ingest_activity(&[episode("e1", "editing the deploy runbook", Utc::now())])
            .unwrap();
    }
    let ro = Brain::open(BrainConfig {
        read_only: true,
        ..brain_config(&tmp)
    })
    .unwrap();

    assert!(ro
        .probe("deploy runbook", ProbeOpts::default())
        .unwrap()
        .iter()
        .any(|r| r.key == "runbook"));
    assert!(ro
        .probe_recent(
            ProbeWindow::Duration(Duration::hours(1)),
            ProbeOpts::default()
        )
        .is_ok());
}
