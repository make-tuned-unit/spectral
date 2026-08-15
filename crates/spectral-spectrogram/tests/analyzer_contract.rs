//! `SpectrogramAnalyzer` — peak selection, config plumbing, and the invariant
//! that the audit path agrees with the real one.
//!
//! `analyze_with_introspection` re-implements the whole dimension pipeline
//! alongside `analyze`. Two copies of one computation is exactly the shape
//! that drifts: a change made in one and not the other leaves the audit trail
//! describing a fingerprint the system never actually stored, and nothing
//! would report it. That invariant is asserted first, over a spread of inputs.

use spectral_ingest::Memory;
use spectral_spectrogram::analyzer::{AnalysisContext, AnalyzerConfig, SpectrogramAnalyzer};

fn memory(id: &str, content: &str) -> Memory {
    Memory {
        id: id.into(),
        key: format!("key-{id}"),
        content: content.into(),
        wing: Some("w".into()),
        hall: Some("fact".into()),
        signal_score: 0.7,
        visibility: "private".into(),
        source: None,
        device_id: None,
        confidence: 1.0,
        created_at: None,
        last_reinforced_at: None,
        episode_id: None,
        compaction_tier: None,
        declarative_density: None,
        description: None,
        description_generated_at: None,
        content_hash: None,
        source_brain_id: None,
        signature: None,
    }
}

/// A spread of inputs chosen to light up different dimensions: a decision, a
/// discovery, a task, an emotional line, a dated line, and an empty one.
const CORPUS: &[&str] = &[
    "we decided to go with postgres after the benchmark",
    "found that the retry loop was the actual bottleneck",
    "need to migrate the staging cluster before friday",
    "honestly this was a frustrating and disappointing week",
    "on 2024-03-04 at 10:00 the incident began in eu-west-1",
    "because the cache expired, the request fell through to origin, which timed out",
    "",
];

// ── the audit path must not drift from the real one ────────────────

/// Every dimension `analyze_with_introspection` reports must equal what
/// `analyze` computes for the same input. If the two implementations ever
/// diverge, the audit trail describes a fingerprint that was never stored.
#[test]
fn introspection_agrees_with_analyze_on_every_dimension() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig::default());
    // A NON-EMPTY wing corpus, deliberately: novelty is the one dimension that
    // reads the context, so an empty corpus makes a divergence in how the two
    // paths use it invisible. An earlier version used the default (empty)
    // context and missed exactly that.
    let ctx = AnalysisContext {
        wing_corpus: CORPUS.join(" "),
    };

    for (i, content) in CORPUS.iter().enumerate() {
        let m = memory(&format!("m{i}"), content);
        let plain = a.analyze(&m, &ctx);
        let (audited, _intro) = a.analyze_with_introspection(&m, &ctx);

        assert_eq!(plain.memory_id, audited.memory_id, "input {i}");
        assert_eq!(
            plain.action_type, audited.action_type,
            "action_type, input {i}"
        );
        for (name, l, r) in [
            (
                "entity_density",
                plain.entity_density,
                audited.entity_density,
            ),
            (
                "decision_polarity",
                plain.decision_polarity,
                audited.decision_polarity,
            ),
            ("causal_depth", plain.causal_depth, audited.causal_depth),
            (
                "emotional_valence",
                plain.emotional_valence,
                audited.emotional_valence,
            ),
            (
                "temporal_specificity",
                plain.temporal_specificity,
                audited.temporal_specificity,
            ),
            ("novelty", plain.novelty, audited.novelty),
        ] {
            assert!(
                (l - r).abs() < 1e-12,
                "{name} differs between analyze and analyze_with_introspection \
                 on input {i}: {l} vs {r}"
            );
        }
        assert_eq!(
            plain.peak_dimensions, audited.peak_dimensions,
            "peak_dimensions differ on input {i}"
        );
    }
}

// ── determinism ────────────────────────────────────────────────────

/// Analysis must be a pure function of (content, corpus): same input, same
/// fingerprint. Only `created_at` may differ.
#[test]
fn analysis_is_deterministic_for_the_same_input() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig::default());
    let ctx = AnalysisContext::default();
    let m = memory("m", CORPUS[0]);

    let one = a.analyze(&m, &ctx);
    let two = a.analyze(&m, &ctx);

    assert_eq!(one.action_type, two.action_type);
    assert_eq!(one.peak_dimensions, two.peak_dimensions);
    assert!((one.entity_density - two.entity_density).abs() < 1e-12);
    assert!((one.novelty - two.novelty).abs() < 1e-12);
}

// ── peak selection ─────────────────────────────────────────────────

/// `peak_dimension_count` must reach peak selection. It is the only knob on
/// the analyzer, so a dropped value silently changes every fingerprint's
/// peak list.
#[test]
fn peak_dimension_count_bounds_the_peak_list() {
    let ctx = AnalysisContext::default();
    let m = memory("m", CORPUS[5]);

    for n in [1usize, 2, 3, 6] {
        let a = SpectrogramAnalyzer::new(AnalyzerConfig {
            peak_dimension_count: n,
        });
        let fp = a.analyze(&m, &ctx);
        assert_eq!(
            fp.peak_dimensions.len(),
            n,
            "peak_dimension_count = {n} produced {} peaks",
            fp.peak_dimensions.len()
        );
    }
}

/// Asking for more peaks than there are dimensions must yield all six rather
/// than panicking on the take().
#[test]
fn requesting_more_peaks_than_dimensions_yields_all_of_them() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig {
        peak_dimension_count: 99,
    });
    let fp = a.analyze(&memory("m", CORPUS[0]), &AnalysisContext::default());
    assert_eq!(fp.peak_dimensions.len(), 6, "there are six dimensions");
    // And no duplicates.
    let unique: std::collections::HashSet<&String> = fp.peak_dimensions.iter().collect();
    assert_eq!(unique.len(), 6, "peak list contains duplicates");
}

/// Zero peaks is degenerate but must not panic.
#[test]
fn zero_peaks_is_an_empty_list_not_a_panic() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig {
        peak_dimension_count: 0,
    });
    let fp = a.analyze(&memory("m", CORPUS[0]), &AnalysisContext::default());
    assert!(fp.peak_dimensions.is_empty());
}

/// Peaks are ranked by ABSOLUTE magnitude, which matters for the two signed
/// dimensions: a strongly NEGATIVE emotional valence is just as much a peak as
/// a positive one. Ranking on the raw value would bury it beneath every
/// non-negative dimension.
///
/// The fixture uses words the scorer actually recognises as negative
/// ("broken", "blocked", "terrible", "crash"), and the test asserts the
/// precondition rather than skipping when it is not met — an earlier version
/// used prose the scorer scored at exactly 0.0 and therefore asserted nothing.
#[test]
fn a_strongly_negative_dimension_still_ranks_as_a_peak() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig {
        peak_dimension_count: 3,
    });
    let ctx = AnalysisContext::default();
    let negative = memory(
        "neg",
        "the build is broken and blocked, a terrible crash, everything broken again",
    );
    let fp = a.analyze(&negative, &ctx);

    assert!(
        fp.emotional_valence < -0.2,
        "fixture precondition failed: expected a strongly negative valence, \
         got {}. Update the fixture rather than letting this test go vacuous.",
        fp.emotional_valence
    );
    assert!(
        fp.peak_dimensions.iter().any(|d| d == "emotional_valence"),
        "a strongly negative emotional_valence ({}) was not ranked as a peak \
         — selection is not using absolute magnitude: {:?}",
        fp.emotional_valence,
        fp.peak_dimensions
    );
}

/// The peak list is drawn from the six known dimension names — a typo in one
/// would be invisible to the type system.
#[test]
fn peak_names_are_drawn_from_the_known_dimension_set() {
    let known = [
        "entity_density",
        "decision_polarity",
        "causal_depth",
        "emotional_valence",
        "temporal_specificity",
        "novelty",
    ];
    let a = SpectrogramAnalyzer::new(AnalyzerConfig::default());
    for (i, content) in CORPUS.iter().enumerate() {
        let fp = a.analyze(&memory("m", content), &AnalysisContext::default());
        for name in &fp.peak_dimensions {
            assert!(
                known.contains(&name.as_str()),
                "unknown peak dimension {name:?} on input {i}"
            );
        }
    }
}

// ── novelty and the wing corpus ────────────────────────────────────

/// Novelty is the one dimension that depends on `AnalysisContext`. Content
/// already present in the wing corpus must score lower than content absent
/// from it — otherwise the corpus is being ignored.
#[test]
fn content_already_in_the_corpus_is_less_novel_than_content_absent_from_it() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig::default());
    let content = "the deploy runbook lives in notion and covers rollback";
    let m = memory("m", content);

    let unseen = a.analyze(&m, &AnalysisContext::default());
    let seen = a.analyze(
        &m,
        &AnalysisContext {
            wing_corpus: content.repeat(3),
        },
    );

    assert!(
        seen.novelty <= unseen.novelty,
        "content already in the corpus scored MORE novel ({}) than the same \
         content against an empty corpus ({}) — the corpus is being ignored",
        seen.novelty,
        unseen.novelty
    );
}

/// Every dimension must stay inside its documented range so downstream
/// thresholds behave. The two signed dimensions are [-1, 1]; the rest [0, 1].
#[test]
fn every_dimension_stays_within_its_range() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig::default());
    for (i, content) in CORPUS.iter().enumerate() {
        let fp = a.analyze(&memory("m", content), &AnalysisContext::default());
        for (name, v) in [
            ("entity_density", fp.entity_density),
            ("causal_depth", fp.causal_depth),
            ("temporal_specificity", fp.temporal_specificity),
            ("novelty", fp.novelty),
        ] {
            assert!(
                (0.0..=1.0).contains(&v),
                "{name} = {v} is outside [0,1] on input {i}"
            );
        }
        for (name, v) in [
            ("decision_polarity", fp.decision_polarity),
            ("emotional_valence", fp.emotional_valence),
        ] {
            assert!(
                (-1.0..=1.0).contains(&v),
                "{name} = {v} is outside [-1,1] on input {i}"
            );
        }
    }
}

/// Empty content must analyse without panicking and produce a well-formed
/// fingerprint — memories can be empty, and this runs on the write path.
#[test]
fn empty_content_analyses_without_panicking() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig::default());
    let fp = a.analyze(&memory("empty", ""), &AnalysisContext::default());
    assert_eq!(fp.memory_id, "empty");
    assert_eq!(fp.peak_dimensions.len(), 3);
}

// ── introspection content ──────────────────────────────────────────

/// The introspection payload must actually describe the decision, not be an
/// empty shell — it is the audit trail.
#[test]
fn introspection_reports_a_rationale_for_every_section() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig::default());
    let m = memory("m", "we decided to go with postgres after the benchmark");
    let (_fp, intro) = a.analyze_with_introspection(&m, &AnalysisContext::default());

    assert!(
        !intro.action_type_rationale.is_empty(),
        "no rationale given for the action type"
    );
    assert!(
        !intro.peak_selection_rationale.is_empty(),
        "no rationale given for peak selection"
    );
    assert!(
        !intro.dimension_calculations.is_empty(),
        "no per-dimension calculations reported"
    );
}

/// A decision line's rationale must name the keyword that triggered it,
/// otherwise the audit trail cannot be checked against the input.
#[test]
fn a_decision_rationale_names_the_matched_keyword() {
    let a = SpectrogramAnalyzer::new(AnalyzerConfig::default());
    let m = memory("m", "we decided to go with postgres");
    let (fp, intro) = a.analyze_with_introspection(&m, &AnalysisContext::default());

    if fp.action_type == spectral_spectrogram::ActionType::Decision {
        assert!(
            intro.action_type_rationale.contains("decided"),
            "the rationale does not name the keyword that matched: {}",
            intro.action_type_rationale
        );
    }
}
