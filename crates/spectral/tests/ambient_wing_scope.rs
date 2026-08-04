//! Ambient wing scope must reach TACT's tier selection.
//!
//! A wing is detected from query text, which requires the user to *name* the
//! project — 12.4% of real agent queries. The agent normally knows its project
//! anyway; `RecognitionContext::focus_wing` is how it says so, and until now
//! that value only fed a rerank boost, never the scoping tier.
//!
//! See `docs/internal/tier1-ungating-result-2026-08-03.md` (R13).

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};
use tempfile::TempDir;

fn brain() -> (TempDir, Brain) {
    let tmp = TempDir::new().unwrap();
    let ontology_path = tmp.path().join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();
    let brain = Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path,
        wing_rules: Some(vec![
            ("polybot".to_string(), "polybot".to_string()),
            ("getladle".to_string(), "getladle".to_string()),
        ]),
        ..Default::default()
    })
    .unwrap();

    for (k, c) in [
        ("p1", "polybot deployment checklist for the release"),
        ("p2", "polybot retry backoff was increased"),
        ("g1", "getladle onboarding flow needs a rewrite"),
        ("g2", "getladle billing reconciliation notes"),
    ] {
        brain.remember(k, c, Visibility::Private).unwrap();
    }
    (tmp, brain)
}

#[test]
fn no_hint_reproduces_prior_behaviour() {
    let (_tmp, brain) = brain();
    // A query that names nothing: wing detection fails, as it does for 87.6%
    // of real queries.
    let q = "what should I look at next";
    let without = brain.tact_retrieve_with_k(q, 10).unwrap();
    let hinted_none = brain.tact_retrieve_with_k_scoped(q, 10, None).unwrap();
    assert_eq!(
        without.iter().map(|h| &h.key).collect::<Vec<_>>(),
        hinted_none.iter().map(|h| &h.key).collect::<Vec<_>>(),
        "a None hint must be byte-identical to the unscoped path"
    );
}

#[test]
fn ambient_hint_supplies_scope_the_query_does_not_name() {
    let (_tmp, brain) = brain();
    let q = "what should I look at next";

    let scoped = brain
        .tact_retrieve_with_k_scoped(q, 10, Some("getladle"))
        .unwrap();

    assert!(!scoped.is_empty(), "ambient scope returned nothing");
    // Everything returned should belong to the hinted wing.
    for h in &scoped {
        assert_eq!(
            h.wing.as_deref(),
            Some("getladle"),
            "hint did not scope retrieval: {} in {:?}",
            h.key,
            h.wing
        );
    }
}

#[test]
fn a_named_wing_in_the_query_overrides_the_ambient_hint() {
    let (_tmp, brain) = brain();
    // The user explicitly says polybot while the agent thinks it is in getladle.
    // The explicit mention must win — ambient context is a fallback, not an
    // override.
    let scoped = brain
        .tact_retrieve_with_k_scoped("polybot retry settings", 10, Some("getladle"))
        .unwrap();
    assert!(!scoped.is_empty());
    assert!(
        scoped.iter().any(|h| h.wing.as_deref() == Some("polybot")),
        "explicit wing lost to the ambient hint: {:?}",
        scoped.iter().map(|h| (&h.key, &h.wing)).collect::<Vec<_>>()
    );
}

#[test]
fn cascade_path_honours_focus_wing() {
    use spectral_graph::cascade_layers::CascadePipelineConfig;
    let (_tmp, brain) = brain();
    let q = "what should I look at next";
    let cfg = CascadePipelineConfig::default();

    let plain = brain
        .recall_cascade_with_pipeline(q, &spectral_cascade::RecognitionContext::empty(), &cfg)
        .unwrap();
    let focused = brain
        .recall_cascade_with_pipeline(
            q,
            &spectral_cascade::RecognitionContext::empty().with_focus_wing("getladle"),
            &cfg,
        )
        .unwrap();

    // An empty context must not change anything; a focused one should.
    assert!(
        !focused.merged_hits.is_empty(),
        "focused recall returned nothing"
    );
    let focused_keys: Vec<&String> = focused.merged_hits.iter().map(|h| &h.key).collect();
    let plain_keys: Vec<&String> = plain.merged_hits.iter().map(|h| &h.key).collect();
    assert!(
        focused_keys != plain_keys || plain_keys.is_empty(),
        "focus_wing did not reach the retrieval path"
    );
}
