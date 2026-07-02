//! Brain-level recognition integration: memories enrolled at write time are
//! recognizable through the public API, with verdicts carrying evidence.

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use spectral_recognition::Verdict;

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
        tact_config: None,
    })
    .unwrap();
    (brain, dir)
}

#[test]
fn remembered_content_is_recognized() {
    let (brain, _dir) = test_brain();
    brain
        .remember(
            "deploy-incident",
            "The staging deploy failed with exit code 137 because the pod was OOMKilled during migration",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "grocery-note",
            "Weekly grocery run planned for Costco with bulk items split among neighbors",
            Visibility::Private,
        )
        .unwrap();

    let r = brain
        .recognize("the deploy failed again — exit 137, pod OOMKilled")
        .unwrap();
    assert_ne!(
        r.verdict,
        Verdict::Novel,
        "re-encounter of a remembered incident must not be novel (familiarity {})",
        r.familiarity
    );
    assert!(!r.evidence.is_empty(), "verdict must carry evidence");
}

#[test]
fn unseen_content_is_novel() {
    let (brain, _dir) = test_brain();
    brain
        .remember(
            "deploy-incident",
            "The staging deploy failed with exit code 137 because the pod was OOMKilled during migration",
            Visibility::Private,
        )
        .unwrap();

    let r = brain
        .recognize("Booked a pottery class for Saturday afternoon in the harbor district")
        .unwrap();
    assert_eq!(r.verdict, Verdict::Novel);
    assert!(r.novelty > 0.8);
}

#[test]
fn recognition_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let ontology_path = dir.path().join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();
    let config = || BrainConfig {
        data_dir: dir.path().to_path_buf(),
        ontology_path: dir.path().join("ontology.toml"),
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
        tact_config: None,
    };
    {
        let brain = Brain::open(config()).unwrap();
        brain
            .remember(
                "unique-fact",
                "Registered domain wealthie-bonds.example with registrar code XR-2291",
                Visibility::Private,
            )
            .unwrap();
    }
    let brain = Brain::open(config()).unwrap();
    let r = brain
        .recognize("domain wealthie-bonds.example registrar XR-2291")
        .unwrap();
    assert_ne!(
        r.verdict,
        Verdict::Novel,
        "recognition index must persist across reopen"
    );
}
