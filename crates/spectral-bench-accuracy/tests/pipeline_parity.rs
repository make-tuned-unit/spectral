//! Parity gate: the library's unified pipeline must reproduce the harness.
//!
//! `spectral::retrieve` exists so a consumer can execute the published
//! configuration in one call. That claim is only worth making if it produces
//! the same thing the benchmark harness produces — otherwise it is a *third*
//! retrieval path rather than the canonical one, which is the problem it was
//! written to solve.
//!
//! This test drives both paths over the same brain and asserts identical
//! retrieved keys and identical rendered lines on the cascade route (the
//! default for every non-Temporal shape — 70% of the held-out set).
//!
//! Scope, deliberately: the harness applies things the library pipeline does
//! not — env-gated associative spreading, BFS expansion, ACT-R rerank,
//! shape-dependent assistant capping. All are OFF by default. Parity is
//! asserted for the **default** configuration, which is the one behind the
//! published number.

use spectral::retrieve::{retrieve, RetrievePlan};
use spectral::{Brain, QuestionShape, RetrievalRoute, Visibility};
use spectral_bench_accuracy::retrieval::{self, RetrievalConfig};
use spectral_graph::brain::BrainConfig;
use tempfile::TempDir;

/// Seed a brain, then hand back both handles onto the same directory: the
/// facade `Brain` the library pipeline takes, and the `spectral_graph` one the
/// harness takes. Reads only, so two handles on one WAL database is fine.
fn seeded_brain() -> (TempDir, Brain, spectral_graph::brain::Brain) {
    let tmp = TempDir::new().unwrap();
    let ontology_path = tmp.path().join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();
    let brain = Brain::open(tmp.path()).unwrap();

    let turns: &[(&str, &str)] = &[
        (
            "s1:turn:0:user",
            "I switched my main laptop to the framework 13 for repairability",
        ),
        ("s1:turn:1:assistant", "Got it."),
        (
            "s1:turn:2:assistant",
            "Noted — the Framework 13 is your main laptop now, chosen for repairability reasons.",
        ),
        (
            "s2:turn:0:user",
            "the framework laptop battery life has been disappointing lately",
        ),
        (
            "s2:turn:1:assistant",
            "That is a known tradeoff on the Framework 13 with the older mainboard.",
        ),
        (
            "s3:turn:0:user",
            "I ran 5 miles on Tuesday near the office before the standup",
        ),
        (
            "s3:turn:1:user",
            "my partner Dana prefers oat milk in her coffee every morning",
        ),
        (
            "s4:turn:0:user",
            "we agreed to move the launch to the second week of March",
        ),
        (
            "s4:turn:1:assistant",
            "Understood, the launch moves to the second week of March.",
        ),
    ];
    for (k, c) in turns {
        brain.remember(k, c, Visibility::Private).unwrap();
    }

    let graph_brain = spectral_graph::brain::Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path,
        ..Default::default()
    })
    .unwrap();
    (tmp, brain, graph_brain)
}

/// Questions that route to Cascade — the default path.
fn cascade_questions() -> Vec<&'static str> {
    [
        "what laptop am I using",
        "how many miles did I run",
        "who prefers oat milk",
        "what did we agree about the launch",
        "tell me about the battery",
    ]
    .into_iter()
    .filter(|q| QuestionShape::classify(q).retrieval_route() == RetrievalRoute::Cascade)
    .collect()
}

#[test]
fn library_pipeline_matches_the_harness_on_the_cascade_route() {
    let (_tmp, brain, graph_brain) = seeded_brain();
    let config = RetrievalConfig::default();
    let questions = cascade_questions();
    assert!(
        !questions.is_empty(),
        "no cascade-routed questions in the fixture"
    );

    for q in questions {
        // Harness path — what the benchmark actually runs.
        let (harness_lines, harness_hits, _telemetry) =
            retrieval::retrieve_cascade(&graph_brain, q, &config, None).unwrap();

        // Library path — what a consumer runs.
        let plan = RetrievePlan::v1(q, Visibility::Private);
        let lib = retrieve(&brain, q, &plan).unwrap();

        let harness_keys: Vec<&str> = harness_hits.iter().map(|h| h.key.as_str()).collect();
        let lib_keys: Vec<&str> = lib.hits.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(lib_keys, harness_keys, "retrieved keys diverged for {q:?}");
        assert_eq!(
            lib.lines, harness_lines,
            "rendered context diverged for {q:?}"
        );
    }
}

#[test]
fn library_pipeline_picks_the_same_route_as_the_harness() {
    for q in [
        "when did I switch laptops",
        "what laptop am I using",
        "how many miles did I run",
        "any tips for battery life",
    ] {
        let plan = RetrievePlan::v1(q, Visibility::Private);
        let shape = QuestionShape::classify(q);
        assert_eq!(plan.shape, shape, "shape diverged for {q:?}");
        assert_eq!(
            plan.route,
            shape.retrieval_route(),
            "route diverged for {q:?}"
        );
    }
}

#[test]
fn the_published_plan_enables_no_unproven_levers() {
    // Guards against a future default flip smuggling a measured-null lever
    // into the configuration behind the published number.
    let plan = RetrievePlan::v1("what laptop am I using", Visibility::Private);
    assert!(
        !plan.answerability.enabled,
        "answerability is REFUTED (3 runs) and must never default on"
    );
    assert_eq!(
        plan.render.session_order,
        spectral::SessionOrder::Chronological,
        "published rendering is chronological"
    );
    assert!(
        plan.render.cap_frac.is_none(),
        "assistant cap was measured at -15pp"
    );
    assert!(
        !plan.render.show_descriptions,
        "descriptions measured no lift"
    );
    assert!(
        !plan.render.relative_offsets,
        "offsets were not in the published run"
    );
}
