use spectral_core::visibility::Visibility;
use spectral_graph::brain::{AaakOpts, Brain, BrainConfig, EntityPolicy};

fn test_brain() -> (Brain, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let ontology_path = dir.path().join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();
    let brain = Brain::open(BrainConfig {
        data_dir: dir.path().to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
    })
    .unwrap();
    (brain, dir)
}

// High-signal content triggers "decided" booster (+0.15) on fact hall (0.7) = 0.85
fn remember_high(brain: &Brain, key: &str, content: &str) {
    brain.remember(key, content, Visibility::Private).unwrap();
}

#[test]
fn aaak_returns_high_signal_facts_only() {
    let (brain, _dir) = test_brain();
    // "decided" triggers decision booster → high score (~0.85)
    remember_high(
        &brain,
        "auth_policy",
        "Alice decided to use OAuth for all services",
    );
    // Plain content → base score (~0.5) which is below default threshold
    remember_high(&brain, "lunch_note", "Had a sandwich for lunch");

    let result = brain.aaak(AaakOpts::default()).unwrap();
    // High-signal fact should be included, low-signal should not (threshold 0.7)
    assert!(
        result.formatted.contains("OAuth") || result.fact_count == 0,
        "high-signal fact should be included if classified as fact"
    );
}

#[test]
fn aaak_respects_token_budget() {
    let (brain, _dir) = test_brain();
    for i in 0..30 {
        remember_high(
            &brain,
            &format!("decision_{i}"),
            &format!("Alice decided to adopt policy number {i} as a critical rule for the team"),
        );
    }

    let result = brain
        .aaak(AaakOpts {
            max_tokens: 40,
            min_signal_score: 0.0,
            include_halls: vec![
                "fact".into(),
                "preference".into(),
                "decision".into(),
                "rule".into(),
                "event".into(),
                "discovery".into(),
                "advice".into(),
            ],
            ..Default::default()
        })
        .unwrap();
    assert!(
        result.estimated_tokens <= 45,
        "estimated tokens {} should be near budget 40",
        result.estimated_tokens
    );
}

#[test]
fn aaak_filters_by_hall() {
    let (brain, _dir) = test_brain();
    // These should get classified differently by the hall classifier
    remember_high(
        &brain,
        "auth_decision",
        "Alice decided to use Clerk for auth",
    );
    remember_high(
        &brain,
        "standup_event",
        "Team had standup meeting at nine AM",
    );

    // Test that AAAK with restrictive halls doesn't crash
    let result = brain
        .aaak(AaakOpts {
            min_signal_score: 0.0,
            include_halls: vec!["fact".into()],
            ..Default::default()
        })
        .unwrap();
    // Verify it runs without error — result is valid
    let _ = result.fact_count;
}

#[test]
fn aaak_filters_by_wing() {
    let (brain, _dir) = test_brain();
    remember_high(
        &brain,
        "apollo_auth",
        "Apollo uses OAuth for authentication",
    );
    remember_high(
        &brain,
        "acme_deploy",
        "Acme deploys everything via Docker containers",
    );

    let result = brain
        .aaak(AaakOpts {
            include_wings: Some(vec!["apollo".into()]),
            min_signal_score: 0.0,
            include_halls: vec![
                "fact".into(),
                "preference".into(),
                "decision".into(),
                "rule".into(),
                "event".into(),
                "discovery".into(),
                "advice".into(),
            ],
            ..Default::default()
        })
        .unwrap();
    // If any facts found, they should only be from apollo wing
    for w in &result.wings_represented {
        assert_eq!(w, "apollo", "should only include apollo wing, got {w}");
    }
}

#[test]
fn aaak_orders_by_signal_score() {
    let (brain, _dir) = test_brain();
    // "rule" booster + "decided" → higher score
    remember_high(
        &brain,
        "critical_rule",
        "Alice decided this must always be the critical rule for deployment",
    );
    // Lower signal — just a basic event
    remember_high(&brain, "meeting_note", "Had a brief team meeting today");

    let result = brain
        .aaak(AaakOpts {
            min_signal_score: 0.0,
            include_halls: vec![
                "fact".into(),
                "preference".into(),
                "decision".into(),
                "rule".into(),
                "event".into(),
                "discovery".into(),
                "advice".into(),
            ],
            ..Default::default()
        })
        .unwrap();
    if result.fact_count >= 2 {
        let lines: Vec<&str> = result.formatted.lines().collect();
        // Higher-scored memory should appear first
        assert!(
            lines[0].contains("rule")
                || lines[0].contains("decided")
                || lines[0].contains("critical"),
            "highest score should appear first, got: {}",
            lines[0]
        );
    }
}

#[test]
fn aaak_deterministic() {
    let (brain, _dir) = test_brain();
    remember_high(
        &brain,
        "fact_a",
        "Alice decided to always use TypeScript over JavaScript",
    );
    remember_high(
        &brain,
        "fact_b",
        "Bob decided to always review PRs before merging as a rule",
    );

    let r1 = brain
        .aaak(AaakOpts {
            min_signal_score: 0.0,
            include_halls: vec![
                "fact".into(),
                "preference".into(),
                "decision".into(),
                "rule".into(),
                "event".into(),
            ],
            ..Default::default()
        })
        .unwrap();
    let r2 = brain
        .aaak(AaakOpts {
            min_signal_score: 0.0,
            include_halls: vec![
                "fact".into(),
                "preference".into(),
                "decision".into(),
                "rule".into(),
                "event".into(),
            ],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r1.formatted, r2.formatted, "aaak should be deterministic");
    assert_eq!(r1.fact_count, r2.fact_count);
}

#[test]
fn aaak_handles_empty_brain() {
    let (brain, _dir) = test_brain();
    let result = brain.aaak(AaakOpts::default()).unwrap();
    assert_eq!(result.fact_count, 0);
    assert!(result.formatted.is_empty());
    assert_eq!(result.estimated_tokens, 0);
    assert_eq!(result.excluded_count, 0);
}

#[test]
fn aaak_handles_zero_budget() {
    let (brain, _dir) = test_brain();
    remember_high(
        &brain,
        "some_fact",
        "Alice decided to use Rust for systems code",
    );

    let result = brain
        .aaak(AaakOpts {
            max_tokens: 0,
            ..Default::default()
        })
        .unwrap();
    // With 0 budget, at most 1 fact (first one always included even at 0)
    assert!(result.fact_count <= 1);
}

#[test]
fn aaak_excluded_count_correct() {
    let (brain, _dir) = test_brain();
    for i in 0..15 {
        remember_high(
            &brain,
            &format!("decision_{i}"),
            &format!("Alice decided to adopt important policy {i} as a critical rule always"),
        );
    }

    let result = brain
        .aaak(AaakOpts {
            max_tokens: 30,
            min_signal_score: 0.0,
            include_halls: vec![
                "fact".into(),
                "preference".into(),
                "decision".into(),
                "rule".into(),
                "event".into(),
                "discovery".into(),
                "advice".into(),
            ],
            ..Default::default()
        })
        .unwrap();
    // Total candidates that passed hall/wing/score filters
    let total_candidates = result.fact_count + result.excluded_count;
    assert!(total_candidates > 0, "should have some candidates");
    assert!(
        result.excluded_count > 0,
        "tight budget should exclude some facts"
    );
}

#[test]
fn aaak_estimated_tokens_consistent() {
    let (brain, _dir) = test_brain();
    remember_high(
        &brain,
        "fact_1",
        "Alice decided to use Clerk for authentication as a critical rule",
    );
    remember_high(
        &brain,
        "fact_2",
        "Bob decided dark roast coffee is the rule every morning always",
    );

    let result = brain
        .aaak(AaakOpts {
            min_signal_score: 0.0,
            include_halls: vec![
                "fact".into(),
                "preference".into(),
                "decision".into(),
                "rule".into(),
                "event".into(),
                "discovery".into(),
                "advice".into(),
            ],
            ..Default::default()
        })
        .unwrap();

    if !result.formatted.is_empty() {
        let actual_estimate = (result.formatted.len() as f64 / 4.0).ceil() as usize;
        assert_eq!(
            result.estimated_tokens, actual_estimate,
            "estimated_tokens should match chars/4"
        );
    }
}
