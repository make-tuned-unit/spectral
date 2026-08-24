//! R38 — the spectrogram wire: fingerprints computed over content+description,
//! and resonance used as a lookup.
//!
//! Both exist to make a hypothesis testable that had never been tested:
//! `SpectrogramAnalyzer::analyze` never read `Memory::description`, so every
//! spectrogram ever measured was content-only even on a 96.6%-enriched brain.
//! The measured verdict was that enrichment does NOT rescue the spectrogram
//! (docs/internal/r38-resonance-enriched-result-2026-08-18.md) — but the code
//! ships behind `spectrogram-legacy`, and shipping code gets tests.
#![cfg(feature = "spectrogram-legacy")]

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use tempfile::TempDir;

fn brain(tmp: &TempDir) -> Brain {
    let onto = tmp.path().join("ontology.toml");
    std::fs::write(&onto, "version = 1\n").unwrap();
    Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: onto,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: true,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
        ..Default::default()
    })
    .unwrap()
}

fn remember(b: &Brain, key: &str, content: &str) -> String {
    b.remember_with(
        key,
        content,
        RememberOpts {
            visibility: Visibility::Private,
            wing: Some("ops".into()),
            ..Default::default()
        },
    )
    .unwrap()
    .memory_id
}

/// The count returned must be the number of memories actually re-fingerprinted
/// — described ones only, and every one of them.
///
/// A caller uses this number to decide whether the enrichment reached the
/// analyzer at all; a wrong count is indistinguishable from a wire that did
/// not fire, which is the exact failure R35 spent an experiment discovering.
#[test]
fn refingerprint_counts_exactly_the_described_memories() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    let a = remember(
        &b,
        "k-a",
        "The ledger export finished in 4471ms writing 219 rows",
    );
    let c = remember(
        &b,
        "k-c",
        "The vault archive was verified overnight without errors",
    );
    let _undescribed = remember(&b, "k-u", "A third memory that never gets a description");

    assert_eq!(
        b.refingerprint_from_descriptions(1000).unwrap(),
        0,
        "nothing is described yet, so nothing is re-fingerprinted"
    );

    b.set_description(&a, "ledger export: 4471ms, 219 rows, archive-west")
        .unwrap();
    assert_eq!(
        b.refingerprint_from_descriptions(1000).unwrap(),
        1,
        "exactly the one described memory"
    );

    b.set_description(&c, "vault archive verified, no errors, overnight run")
        .unwrap();
    assert_eq!(
        b.refingerprint_from_descriptions(1000).unwrap(),
        2,
        "both described memories — the undescribed one is never counted"
    );
}

/// Resonance must actually return the other memory, exclude the seed itself,
/// and honour the result cap.
#[test]
fn resonant_memory_ids_returns_neighbours_excludes_the_seed_and_honours_the_cap() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    // Same shape of statement, so their fingerprints land close together.
    let seed = remember(
        &b,
        "k1",
        "Decided to move the deploy window to Tuesday evening",
    );
    remember(
        &b,
        "k2",
        "Decided to move the backup window to Thursday evening",
    );
    remember(
        &b,
        "k3",
        "Decided to move the review window to Monday evening",
    );

    let tol = spectral_spectrogram::matching::MatchTolerances::default();
    let all = b
        .resonant_memory_ids(std::slice::from_ref(&seed), 10, &tol)
        .unwrap();
    assert!(
        !all.is_empty(),
        "similar decisions should resonate; got nothing"
    );
    assert!(
        !all.iter().any(|(id, _)| id == &seed),
        "the seed must never be returned as its own neighbour"
    );
    assert!(
        all.iter().all(|(_, score)| *score > 0.0),
        "every returned match carries a resonance score"
    );

    let capped = b
        .resonant_memory_ids(std::slice::from_ref(&seed), 1, &tol)
        .unwrap();
    assert!(capped.len() <= 1, "max_results must cap the result set");

    // Degenerate inputs answer emptily rather than erroring.
    assert!(b.resonant_memory_ids(&[], 10, &tol).unwrap().is_empty());
    assert!(b
        .resonant_memory_ids(std::slice::from_ref(&seed), 0, &tol)
        .unwrap()
        .is_empty());
}
