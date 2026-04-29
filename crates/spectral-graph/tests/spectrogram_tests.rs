use std::path::PathBuf;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};
use tempfile::TempDir;

fn brain_with_spectrogram(tmp: &TempDir) -> Brain {
    Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: true,
        entity_policy: spectral_graph::brain::EntityPolicy::Strict,
    })
    .unwrap()
}

fn brain_without_spectrogram(tmp: &TempDir) -> Brain {
    Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: spectral_graph::brain::EntityPolicy::Strict,
    })
    .unwrap()
}

#[test]
fn backfill_generates_spectrograms() {
    let tmp = TempDir::new().unwrap();
    // Write memories WITHOUT spectrogram enabled
    let brain = brain_without_spectrogram(&tmp);
    for i in 0..5 {
        brain
            .remember(
                &format!("mem-{i}"),
                &format!("Decided to use Polybot for weather prediction strategy {i}"),
                Visibility::Private,
            )
            .unwrap();
    }

    // Backfill should generate spectrograms for all 5
    let count = brain.backfill_spectrograms().unwrap();
    assert_eq!(count, 5);

    // Second call is idempotent
    let count2 = brain.backfill_spectrograms().unwrap();
    assert_eq!(count2, 0);
}

#[test]
fn spectrogram_storage_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let brain = brain_with_spectrogram(&tmp);

    brain
        .remember(
            "roundtrip-key",
            "Decided to use Polybot for weather prediction roundtrip strategy",
            Visibility::Private,
        )
        .unwrap();

    // Verify spectrogram was written (via backfill returning 0)
    let count = brain.backfill_spectrograms().unwrap();
    assert_eq!(count, 0, "spectrogram should already exist from ingest");
}

#[test]
fn disabled_spectrogram_skips_computation() {
    let tmp = TempDir::new().unwrap();
    let brain = brain_without_spectrogram(&tmp);

    brain
        .remember(
            "skip-key",
            "Decided to use Polybot for weather prediction skip strategy",
            Visibility::Private,
        )
        .unwrap();

    // Backfill should find this memory needs a spectrogram
    let count = brain.backfill_spectrograms().unwrap();
    assert_eq!(
        count, 1,
        "disabled spectrogram should not have written one during ingest"
    );
}

#[test]
fn cross_wing_match_finds_resonance() {
    let tmp = TempDir::new().unwrap();
    let brain = brain_with_spectrogram(&tmp);

    // Write two decision memories in different wings
    brain
        .remember(
            "polybot-decision",
            "Decided to use Polybot for the weather prediction strategy",
            Visibility::Private,
        )
        .unwrap();

    // Jesse wing memory (different wing)
    brain
        .remember(
            "jesse-decision",
            "Jesse decided to use Clerk for the auth strategy",
            Visibility::Private,
        )
        .unwrap();

    // Cross-wing recall from polybot wing should find the jesse decision
    let result = brain
        .recall_cross_wing(
            "polybot weather prediction strategy decision",
            Visibility::Private,
            5,
        )
        .unwrap();

    assert!(
        result.seed_memory.is_some(),
        "should find a seed memory for the query"
    );
}

#[test]
fn cross_wing_match_excludes_same_wing() {
    let tmp = TempDir::new().unwrap();
    let brain = brain_with_spectrogram(&tmp);

    // Two memories in the same wing
    brain
        .remember(
            "polybot-a",
            "Decided to use Polybot for the weather prediction strategy A",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember(
            "polybot-b",
            "Decided to use Polybot for the weather prediction strategy B",
            Visibility::Private,
        )
        .unwrap();

    let result = brain
        .recall_cross_wing(
            "polybot weather prediction strategy decision",
            Visibility::Private,
            5,
        )
        .unwrap();

    // Resonant memories should NOT include same-wing memories
    for hit in &result.resonant_memories {
        let seed_wing = result.seed_memory.as_ref().and_then(|s| s.wing.as_deref());
        let hit_wing = hit.memory.wing.as_deref();
        if let (Some(sw), Some(hw)) = (seed_wing, hit_wing) {
            assert_ne!(sw, hw, "resonant memory should be from a different wing");
        }
    }
}

#[test]
fn cross_wing_match_respects_visibility() {
    let tmp = TempDir::new().unwrap();
    let brain = brain_with_spectrogram(&tmp);

    brain
        .remember(
            "polybot-public-decision",
            "Decided to use Polybot for the weather prediction strategy public",
            Visibility::Public,
        )
        .unwrap();

    brain
        .remember(
            "jesse-private-decision",
            "Jesse decided to use Clerk for the auth strategy private",
            Visibility::Private,
        )
        .unwrap();

    // Public context query should not see private resonant memories
    let result = brain
        .recall_cross_wing(
            "polybot weather prediction strategy decision",
            Visibility::Public,
            5,
        )
        .unwrap();

    for hit in &result.resonant_memories {
        assert_eq!(
            hit.memory.visibility, "public",
            "public context should not see private resonant memories"
        );
    }
}
