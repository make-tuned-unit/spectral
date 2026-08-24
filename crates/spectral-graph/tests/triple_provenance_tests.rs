//! R43 — a triple extracted FROM a memory carries that memory as its source.
//!
//! Every triple already had a `source_doc_id` field on its provenance and
//! `assert`/`assert_typed` always wrote `None`, so a relation extracted by an
//! enrichment pass had no path back to the evidence that produced it. These
//! pin the round trip and, as importantly, that an UNSOURCED assertion is not
//! attributed to anything — a provenance link that over-claims is worse than
//! none, because it makes a bad extraction look evidenced.

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use std::path::PathBuf;
use tempfile::TempDir;

fn brain(tmp: &TempDir) -> Brain {
    Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::AutoCreate,
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
            ..Default::default()
        },
    )
    .unwrap()
    .memory_id
}

const SUBJ: (&str, &str) = ("person", "Mark Smith");
const OBJ: (&str, &str) = ("topic", "library science");

#[test]
fn a_sourced_triple_is_recoverable_by_its_memory_and_only_by_that_memory() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    let src = remember(
        &b,
        "k-src",
        "Mark Smith studies library science at the college",
    );
    let other = remember(
        &b,
        "k-other",
        "Unrelated: the office plant was watered on Tuesday",
    );

    let r = b
        .assert_typed_from(&src, SUBJ, "studies", OBJ, 0.6, Visibility::Private)
        .unwrap();
    assert!(r.triple_written);

    let from_src = b.triples_from_memory(&src).unwrap();
    assert_eq!(from_src.len(), 1, "exactly the triple sourced from k-src");
    assert_eq!(from_src[0].predicate, "studies");
    assert_eq!(
        from_src[0].source_doc_id,
        Some(
            *blake3::hash("Mark Smith studies library science at the college".as_bytes())
                .as_bytes()
        ),
        "provenance must be blake3 of the SOURCE MEMORY's content"
    );
    assert!(
        b.triples_from_memory(&other).unwrap().is_empty(),
        "an unrelated memory must not inherit it"
    );
}

#[test]
fn an_unsourced_assertion_is_attributed_to_no_memory() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    let src = remember(
        &b,
        "k-src",
        "Mark Smith studies library science at the college",
    );

    b.assert_typed(SUBJ, "studies", OBJ, 0.9, Visibility::Private)
        .unwrap();
    assert!(
        b.triples_from_memory(&src).unwrap().is_empty(),
        "plain assert_typed must leave source_doc_id None, not borrow a memory"
    );

    // And once a sourced one exists, only it is returned — the unsourced
    // sibling on the same predicate must not be swept in.
    b.assert_typed_from(&src, SUBJ, "studies", OBJ, 0.6, Visibility::Private)
        .unwrap();
    let from_src = b.triples_from_memory(&src).unwrap();
    assert_eq!(
        from_src.len(),
        1,
        "only the sourced assertion is attributed, got {from_src:?}"
    );
}

#[test]
fn an_unknown_memory_id_is_an_error_not_an_empty_answer() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    assert!(
        b.assert_typed_from("no-such-id", SUBJ, "studies", OBJ, 0.6, Visibility::Private)
            .is_err(),
        "asserting FROM a memory that does not exist must fail loudly"
    );
    assert!(
        b.triples_from_memory("no-such-id").is_err(),
        "querying an unknown memory must not look like 'this memory sourced nothing'"
    );
}
