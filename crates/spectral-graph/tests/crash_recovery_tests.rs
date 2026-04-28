//! Crash-recovery tests for Spectral Brain.
//!
//! These tests verify what happens when a brain is dropped mid-operation
//! and then reopened. The goal is to prove either that writes are atomic
//! (all-or-nothing) or to document the exact failure mode.
//!
//! # Findings
//!
//! ## SQLite (memory store)
//! SQLite with WAL mode provides per-statement atomicity. Each INSERT is
//! atomic. However, `MemoryStore::write()` does multiple INSERTs (one for
//! the memory, then one per fingerprint) without an explicit transaction.
//! If the process crashes between them, the memory exists but some
//! fingerprints may be missing. This is the "orphan memory" scenario.
//!
//! In practice, SQLite WAL + `PRAGMA synchronous = NORMAL` means data
//! written before the crash is durable. The risk is partial fingerprint
//! sets, not data corruption.
//!
//! ## Kuzu (graph store)
//! `Brain::assert()` does three separate Kuzu operations (upsert subject,
//! upsert object, insert triple). Each is its own connection+statement.
//! Kuzu doesn't expose user-facing transactions in its Rust API. If the
//! process crashes between operations, entities may exist without their
//! connecting triple. This is a "dangling entity" scenario.
//!
//! Since `upsert_entity` is idempotent, re-running the assert after
//! recovery will fix the state.

use std::path::PathBuf;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};
use tempfile::TempDir;

fn brain_config(tmp: &TempDir) -> BrainConfig {
    BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
    }
}

/// Verify that a brain can be reopened after an abrupt drop during remember().
///
/// Pattern: write a memory, drop the brain without graceful shutdown,
/// reopen and verify the memory landed. SQLite WAL mode should ensure
/// completed writes survive process death.
#[test]
fn brain_reopens_after_drop_during_remember() {
    let tmp = TempDir::new().unwrap();

    // First session: write a memory, then drop abruptly.
    {
        let brain = Brain::open(brain_config(&tmp)).unwrap();
        brain
            .remember(
                "crash-test",
                "Alice decided to use Clerk for auth",
                Visibility::Private,
            )
            .unwrap();
        // Brain dropped here — no graceful shutdown.
    }

    // Second session: reopen and verify state.
    {
        let brain = Brain::open(brain_config(&tmp)).unwrap();
        let result = brain
            .recall("what did Alice decide about auth", Visibility::Private)
            .unwrap();

        // The memory should have survived the drop. SQLite WAL ensures
        // completed writes are durable even without explicit close.
        assert!(
            !result.memory_hits.is_empty(),
            "Memory written before drop should survive reopen"
        );
        assert_eq!(result.memory_hits[0].key, "crash-test");
    }
}

/// Verify that a brain can be reopened after an abrupt drop during assert().
///
/// Brain::assert() does three Kuzu operations: upsert subject, upsert
/// object, insert triple. If all three complete before drop, the data
/// should survive reopen.
#[test]
fn brain_reopens_after_drop_during_assert() {
    let tmp = TempDir::new().unwrap();

    {
        let brain = Brain::open(brain_config(&tmp)).unwrap();
        brain
            .assert("Mark", "studies", "Library", 0.9, Visibility::Private)
            .unwrap();
        // Drop without graceful shutdown.
    }

    {
        let brain = Brain::open(brain_config(&tmp)).unwrap();
        let result = brain.recall_graph("Mark", Visibility::Private).unwrap();

        assert!(
            !result.triples.is_empty(),
            "Triple written before drop should survive reopen"
        );
        assert_eq!(result.triples[0].predicate, "studies");
    }
}

/// Verify that a brain can be reopened after an abrupt drop during
/// ingest_document(). Document ingestion writes a Document node plus
/// one Mentions edge per matched entity — multiple Kuzu operations.
#[test]
fn brain_reopens_after_drop_during_ingest_document() {
    let tmp = TempDir::new().unwrap();

    {
        let brain = Brain::open(brain_config(&tmp)).unwrap();
        brain
            .ingest_document(
                "test.txt",
                "Carol works on Spectral every day",
                Visibility::Private,
            )
            .unwrap();
        // Drop.
    }

    {
        let brain = Brain::open(brain_config(&tmp)).unwrap();
        // Re-ingesting the same document should not error (idempotent upserts).
        let result = brain
            .ingest_document(
                "test.txt",
                "Carol works on Spectral every day",
                Visibility::Private,
            )
            .unwrap();

        // The document ID is deterministic (blake3 of content).
        assert_eq!(result.document_id.len(), 32);
        // Entities should be found by the canonicalizer.
        assert!(result.matched.len() >= 2);
    }
}

/// Test what happens when a memory is written but fingerprints are
/// partially missing ("orphan memory").
///
/// LIMITATION: SqliteStore::write() does not wrap the memory INSERT and
/// fingerprint INSERTs in a single transaction. A crash between the two
/// leaves a memory without all its fingerprints. This doesn't cause
/// corruption — the memory is still retrievable via FTS — but fingerprint-
/// based retrieval may miss it until the next ingest pairs it again.
///
/// This test verifies the brain handles this gracefully on reopen:
/// the orphan memory is readable, and a subsequent remember() with a
/// different key can still generate fingerprints pairing against it.
#[test]
fn fingerprint_orphan_detection() {
    let tmp = TempDir::new().unwrap();

    {
        let brain = Brain::open(brain_config(&tmp)).unwrap();
        // Write a memory — its fingerprints depend on existing peers.
        // First memory in a wing has zero fingerprints (no peers yet),
        // so there's no orphan risk on the first write.
        brain
            .remember(
                "first-memory",
                "Apollo weather prediction strategy decided",
                Visibility::Private,
            )
            .unwrap();
        // Second memory should generate fingerprints pairing with first.
        brain
            .remember(
                "second-memory",
                "Apollo weather engine has a known bug that crashes",
                Visibility::Private,
            )
            .unwrap();
    }

    // Reopen and verify both memories exist.
    {
        let brain = Brain::open(brain_config(&tmp)).unwrap();

        // LIMITATION: We can't easily test a partial-fingerprint state
        // without injecting a fault into SqliteStore::write(). Instead,
        // we verify the graceful case: both memories survived, and a
        // third memory can still generate fingerprints pairing with them.
        let r = brain
            .remember(
                "third-memory",
                "Apollo weather accuracy improved after the fix",
                Visibility::Private,
            )
            .unwrap();

        // The third memory should have fingerprints pairing with the
        // two existing peers in the "apollo" wing.
        assert!(
            r.fingerprints_created >= 1,
            "Third memory should pair with existing peers; got {} fingerprints",
            r.fingerprints_created
        );
    }
}
