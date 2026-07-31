//! Deletion guarantees, proven through the PUBLIC facade.
//!
//! `docs/DELETION_GUARANTEES.md` states its claims in terms of `Brain::forget()`
//! and `Brain::vacuum()`. A reader takes that to mean `spectral::Brain` — the
//! only surface the consumer (Permagent) uses. The deep proof suite
//! (`spectral-graph/tests/deletion_guarantees.rs`, claims D1–D5) exercises the
//! *inner* `spectral_graph::brain::Brain`, so until now nothing pinned that the
//! documented erasure path was reachable at all from the public API — and
//! `vacuum` in fact was not exposed there.
//!
//! This file is the facade-level gate for that boundary. It deliberately does
//! not duplicate D1–D5; it asserts only that the erasure sequence the doc
//! promises is callable on `spectral::Brain` and physically erases.

use std::path::PathBuf;

use spectral::{Brain, RecallTopKConfig, Visibility};
use tempfile::TempDir;

/// Unique, all-lowercase sentinel — lowercase-only so every substrate
/// transform that lowercases (FTS tokenizers, recognition feature extraction)
/// preserves the exact byte sequence.
const SENTINEL: &str = "sentinelpub3wq8m2z";

fn db_files(dir: &std::path::Path) -> Vec<PathBuf> {
    ["memory.db", "recognition.db", "graph.sqlite"]
        .iter()
        .flat_map(|base| {
            ["", "-wal", "-shm"]
                .iter()
                .map(move |suf| dir.join(format!("{base}{suf}")))
        })
        .filter(|p| p.exists())
        .collect()
}

fn files_containing_sentinel(files: &[PathBuf]) -> Vec<PathBuf> {
    let needle = SENTINEL.as_bytes();
    files
        .iter()
        .filter(|p| {
            std::fs::read(p)
                .unwrap()
                .windows(needle.len())
                .any(|w| w == needle)
        })
        .cloned()
        .collect()
}

/// The documented erasure path — `forget` then `vacuum` — must be reachable on
/// `spectral::Brain` and must physically remove the bytes.
#[test]
fn public_forget_then_vacuum_physically_erases() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();

    let content = format!(
        "quarterly retention memo {SENTINEL} lists the shredding schedule for tape backups"
    );
    brain
        .remember("pub-victim", &content, Visibility::Private)
        .unwrap();
    brain
        .remember(
            "pub-bystander",
            "the tape backup cabinet was reorganised",
            Visibility::Private,
        )
        .unwrap();

    let report = brain.forget("pub-victim").unwrap();
    assert!(
        report.fully_forgotten(),
        "public forget must report full deletion across substrates"
    );

    // Pre-vacuum the bytes are EXPECTED to persist — this is the boundary the
    // doc draws, and it proves the scan can find the needle at all.
    let dirty = files_containing_sentinel(&db_files(tmp.path()));
    assert!(
        !dirty.is_empty(),
        "logically-deleted bytes should persist pre-vacuum (else this scan proves nothing)"
    );

    brain.vacuum().unwrap();
    drop(brain);

    let residue = files_containing_sentinel(&db_files(tmp.path()));
    assert!(
        residue.is_empty(),
        "sentinel bytes must be physically absent after public forget + vacuum; found in {residue:?}"
    );

    // Compaction must not damage unrelated memories.
    let brain = Brain::open(tmp.path()).unwrap();
    let hits = brain
        .recall_topk_fts(
            "tape backup cabinet",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();
    assert!(
        hits.iter().any(|h| h.key == "pub-bystander"),
        "vacuum must not damage other memories"
    );
}

/// `vacuum` is safe to call with nothing to erase, and idempotent — the doc
/// presents it as a maintenance operation, not a one-shot.
#[test]
fn public_vacuum_is_safe_and_idempotent() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    brain
        .remember(
            "keep-me",
            "an ordinary retained memory",
            Visibility::Private,
        )
        .unwrap();

    brain.vacuum().unwrap();
    brain.vacuum().unwrap();

    let hits = brain
        .recall_topk_fts(
            "ordinary retained memory",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();
    assert!(
        hits.iter().any(|h| h.key == "keep-me"),
        "repeated vacuum must preserve live memories"
    );
}
