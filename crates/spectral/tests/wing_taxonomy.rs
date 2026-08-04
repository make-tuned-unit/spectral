//! The library must not invent a wing taxonomy, and must be able to repair
//! brains that were filed under one.
//!
//! See `docs/internal/wing-taxonomy-2026-08-03.md`.

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};
use tempfile::TempDir;

fn brain_with(rules: Option<Vec<(String, String)>>) -> (TempDir, Brain) {
    let tmp = TempDir::new().unwrap();
    let ontology_path = tmp.path().join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();
    let brain = Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path,
        wing_rules: rules,
        ..Default::default()
    })
    .unwrap();
    (tmp, brain)
}

/// The wing currently stored for `key`.
fn wing_of(brain: &Brain, key: &str) -> String {
    brain
        .list_all_memories(usize::MAX)
        .unwrap()
        .into_iter()
        .find(|m| m.key == key)
        .and_then(|m| m.wing)
        .unwrap_or_default()
}

/// Content that the removed fixtures would have captured.
const FIXTURE_BAIT: &[(&str, &str)] = &[
    ("m1", "Alice likes coffee in the morning"),
    ("m2", "the apollo weather prediction was wrong"),
    ("m3", "acme widget recipe for the feast"),
    ("m4", "polaris volunteer marathon summit"),
];

#[test]
fn default_brain_invents_no_wings() {
    let (_tmp, brain) = brain_with(None);
    for (k, c) in FIXTURE_BAIT {
        brain.remember(k, c, Visibility::Private).unwrap();
    }
    for (k, _) in FIXTURE_BAIT {
        assert_eq!(
            wing_of(&brain, k),
            "general",
            "fixture wing assigned for {k}"
        );
    }
}

#[test]
fn consumer_wings_are_applied() {
    let rules = vec![
        (
            "henry-infra|task runner|deploy".to_string(),
            "henry-infra".to_string(),
        ),
        ("getladle|ladle".to_string(), "getladle".to_string()),
    ];
    let (_tmp, brain) = brain_with(Some(rules));
    brain
        .remember(
            "a",
            "the task runner deploy failed again",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember("b", "ladle onboarding notes", Visibility::Private)
        .unwrap();
    brain
        .remember("c", "unrelated musings", Visibility::Private)
        .unwrap();

    assert_eq!(wing_of(&brain, "a"), "henry-infra");
    assert_eq!(wing_of(&brain, "b"), "getladle");
    assert_eq!(wing_of(&brain, "c"), "general");
}

#[test]
fn reclassify_dry_run_changes_nothing_and_reports_what_would_change() {
    // Ingest under a taxonomy, then ask what a DIFFERENT taxonomy would do.
    let rules = vec![("coffee".to_string(), "beverages".to_string())];
    let (_tmp, brain) = brain_with(Some(rules));
    brain
        .remember(
            "m1",
            "Alice likes coffee in the morning",
            Visibility::Private,
        )
        .unwrap();
    assert_eq!(wing_of(&brain, "m1"), "beverages");

    // Same brain reopened with NO rules: everything should want to be general.
    let report = brain.reclassify_wings(false).unwrap();
    assert!(!report.applied);
    // Dry run must not have written.
    assert_eq!(
        wing_of(&brain, "m1"),
        "beverages",
        "dry run wrote to the database"
    );
    assert_eq!(report.scanned, 1);
    assert_eq!(report.changed(), 0, "same rules should be a no-op");
}

#[test]
fn reclassify_repairs_fixture_polluted_wings() {
    // Simulate a brain ingested under the old fixtures, then repaired.
    let polluted = vec![
        ("alice|coffee".to_string(), "alice".to_string()),
        ("apollo|weather".to_string(), "apollo".to_string()),
    ];
    let tmp = TempDir::new().unwrap();
    let ontology_path = tmp.path().join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();

    {
        let brain = Brain::open(BrainConfig {
            data_dir: tmp.path().to_path_buf(),
            ontology_path: ontology_path.clone(),
            wing_rules: Some(polluted),
            ..Default::default()
        })
        .unwrap();
        for (k, c) in FIXTURE_BAIT {
            brain.remember(k, c, Visibility::Private).unwrap();
        }
        assert_eq!(wing_of(&brain, "m1"), "alice");
    }

    // Reopen with the shipped (empty) taxonomy and repair.
    let brain = Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path,
        wing_rules: None,
        ..Default::default()
    })
    .unwrap();

    let dry = brain.reclassify_wings(false).unwrap();
    assert!(
        dry.changed() >= 2,
        "expected fixture wings to be flagged: {dry:?}"
    );
    let departures = dry.departures_by_wing();
    assert!(departures.contains_key("alice"), "{departures:?}");

    let applied = brain.reclassify_wings(true).unwrap();
    assert!(applied.applied);
    assert_eq!(
        applied.changed(),
        dry.changed(),
        "dry run must predict the apply"
    );

    for (k, _) in FIXTURE_BAIT {
        assert_eq!(
            wing_of(&brain, k),
            "general",
            "fixture wing survived repair for {k}"
        );
    }

    // Idempotent.
    let again = brain.reclassify_wings(true).unwrap();
    assert_eq!(again.changed(), 0, "repair is not idempotent");
}
