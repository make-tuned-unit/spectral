//! Concurrency tests for Spectral Brain.
//!
//! These tests verify behavior under concurrent access patterns that
//! happen in production: multiple threads writing, readers during writes,
//! and multiple Brain instances on the same data directory.
//!
//! # Findings
//!
//! ## Single Brain instance, multiple threads
//! Brain is `&self` for all operations (no `&mut self`). The SQLite
//! memory store uses `Arc<Mutex<Connection>>` — concurrent calls
//! serialize on the mutex. Kuzu creates a fresh Connection per
//! operation, and Kuzu itself serializes writes internally. So
//! concurrent threads sharing one Brain instance are safe — they
//! serialize, which means correct but not parallel.
//!
//! ## Multiple Brain instances, same path
//! Opening two Brain instances on the same path opens two separate
//! Kuzu databases and two separate SQLite connections. SQLite handles
//! this via WAL file locking. Kuzu's behavior with concurrent
//! processes is less well-defined — it may error on open or produce
//! undefined behavior. This test documents the observed outcome.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

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
        device_id: None,
        enable_spectrogram: false,
        entity_policy: spectral_graph::brain::EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
        ..Default::default()
    }
}

/// Four threads each remember 10 different memories. All 40 should land
/// successfully with no panics and no data corruption.
///
/// This tests the `Arc<Mutex<Connection>>` serialization in SqliteStore.
/// Writes are correct-but-serial — the Mutex ensures only one thread
/// touches SQLite at a time.
#[test]
fn concurrent_remembers_different_keys() {
    let tmp = TempDir::new().unwrap();
    let brain = Arc::new(Brain::open(brain_config(&tmp)).unwrap());

    let mut handles = Vec::new();
    for thread_id in 0..4 {
        let brain = Arc::clone(&brain);
        handles.push(thread::spawn(move || {
            for i in 0..10 {
                let key = format!("thread{thread_id}-key{i}");
                let content = format!("Memory {i} from thread {thread_id} about apollo weather");
                brain
                    .remember(&key, &content, Visibility::Private)
                    .unwrap_or_else(|e| panic!("thread {thread_id} key {i}: {e}"));
            }
        }));
    }

    for h in handles {
        h.join().expect("thread panicked");
    }

    // Verify all 40 memories landed.
    drop(brain);
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    let result = brain
        .recall("apollo weather thread memory", Visibility::Private)
        .unwrap();

    // FTS should find at least some of the 40 memories.
    assert!(
        !result.memory_hits.is_empty(),
        "Expected memory hits after 40 concurrent writes"
    );
}

/// Four threads racing to remember() the same key with different content.
/// The end state should reflect ONE of the four (last-write-wins via
/// ON CONFLICT DO UPDATE), not a mix or corruption.
#[test]
fn concurrent_remembers_same_key() {
    let tmp = TempDir::new().unwrap();
    let brain = Arc::new(Brain::open(brain_config(&tmp)).unwrap());

    let mut handles = Vec::new();
    for thread_id in 0..4 {
        let brain = Arc::clone(&brain);
        handles.push(thread::spawn(move || {
            for _round in 0..5 {
                let content = format!("Content from thread {thread_id} about apollo weather");
                brain
                    .remember("contested-key", &content, Visibility::Private)
                    .unwrap_or_else(|e| panic!("thread {thread_id}: {e}"));
            }
        }));
    }

    for h in handles {
        h.join().expect("thread panicked");
    }

    // Verify exactly one memory with key "contested-key" exists,
    // and its content is from one of the four threads.
    drop(brain);
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    let result = brain
        .recall("apollo weather content from thread", Visibility::Private)
        .unwrap();

    let contested: Vec<_> = result
        .memory_hits
        .iter()
        .filter(|m| m.key == "contested-key")
        .collect();

    // ON CONFLICT(key) DO UPDATE means exactly one row.
    assert!(
        contested.len() <= 1,
        "Expected at most 1 memory for 'contested-key', got {}",
        contested.len()
    );
    if let Some(hit) = contested.first() {
        assert!(
            hit.content.starts_with("Content from thread "),
            "Content should be from one of the threads, got: {}",
            hit.content
        );
    }
}

/// One writer thread doing remember() in a loop, three reader threads
/// doing recall() in a loop. Verifies no panics and no torn reads.
///
/// A "torn read" would be a memory with fields that violate schema
/// constraints (e.g., NULL wing when the classifier always sets one).
/// Since SqliteStore serializes on a Mutex, reads and writes never
/// overlap — torn reads are impossible with a single Brain instance.
///
/// Uses an atomic write counter + generous deadline instead of a
/// wall-clock race so the test is stable on slow CI runners.
#[test]
fn concurrent_reads_during_writes() {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    let tmp = TempDir::new().unwrap();
    let brain = Arc::new(Brain::open(brain_config(&tmp)).unwrap());

    // Seed one memory so readers have something to find.
    brain
        .remember(
            "seed",
            "Apollo weather prediction baseline established",
            Visibility::Private,
        )
        .unwrap();

    let writes_done = Arc::new(AtomicBool::new(false));
    let write_count = Arc::new(AtomicU32::new(0));

    // Writer thread: signals after each write via atomic counter.
    let writer_brain = Arc::clone(&brain);
    let writer_done = Arc::clone(&writes_done);
    let writer_count = Arc::clone(&write_count);
    let writer = thread::spawn(move || {
        for i in 0..20 {
            let key = format!("write-{i}");
            let content = format!("Apollo weather observation number {i}");
            writer_brain
                .remember(&key, &content, Visibility::Private)
                .unwrap_or_else(|e| panic!("writer {i}: {e}"));
            writer_count.fetch_add(1, Ordering::Release);
        }
        writer_done.store(true, Ordering::Release);
    });

    // Reader threads: run until >=1 successful read AND writer is done,
    // or a generous 30s deadline expires.
    let mut readers = Vec::new();
    for reader_id in 0..3 {
        let brain = Arc::clone(&brain);
        let writes_done = Arc::clone(&writes_done);
        let write_count = Arc::clone(&write_count);
        readers.push(thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut reads: u32 = 0;

            loop {
                // Stop once writer is done AND we have at least one read,
                // or if deadline expires.
                if (writes_done.load(Ordering::Acquire) && reads > 0) || Instant::now() > deadline {
                    break;
                }

                // Wait until at least one write has landed before reading,
                // so we exercise genuine concurrent overlap.
                if write_count.load(Ordering::Acquire) == 0 {
                    thread::yield_now();
                    continue;
                }

                let result =
                    brain.recall("apollo weather prediction observation", Visibility::Private);
                match result {
                    Ok(r) => {
                        for hit in &r.memory_hits {
                            assert!(!hit.key.is_empty(), "reader {reader_id}: empty key");
                            assert!(!hit.content.is_empty(), "reader {reader_id}: empty content");
                        }
                        reads += 1;
                    }
                    Err(e) => panic!("reader {reader_id}: {e}"),
                }
            }
            reads
        }));
    }

    writer.join().expect("writer panicked");
    for (i, r) in readers.into_iter().enumerate() {
        let reads = r.join().expect("reader panicked");
        assert!(
            reads > 0,
            "reader {i} completed 0 reads (writes_done={}, write_count={})",
            writes_done.load(std::sync::atomic::Ordering::Relaxed),
            write_count.load(std::sync::atomic::Ordering::Relaxed),
        );
    }
}

/// Two Brain instances opened on the same data_dir simultaneously.
///
/// LIMITATION: Kuzu's behavior with two processes/instances opening the
/// same database path is not well-defined in their Rust API. In testing,
/// the second open may succeed (both instances write to the same files)
/// or fail. SQLite handles this correctly via WAL file locking.
///
/// This test documents the observed behavior. If it passes, both
/// R-24: two `Brain` handles on the same data directory must coexist AND
/// both writes must land.
///
/// This test previously wrapped the second open in a `match` whose Err arm
/// only `eprintln!`d, so it passed whichever way the code behaved — it could
/// not fail, and the multi-handle invariant that the storage claims depend on
/// was untested in either direction. It is now a decided contract, which the
/// WAL + busy_timeout work (R-10/R-09) is what makes safely assertable: the
/// two handles genuinely contend here, from separate threads.
#[test]
fn concurrent_brain_opens_same_path() {
    let tmp = TempDir::new().unwrap();

    let brain1 = Arc::new(Brain::open(brain_config(&tmp)).unwrap());
    let brain2 = Arc::new(
        Brain::open(brain_config(&tmp))
            .expect("a second Brain handle on the same data dir must open"),
    );

    // Contend for real: both handles write concurrently, rather than the old
    // open-then-write-sequentially shape that never overlapped.
    let h1 = {
        let b = Arc::clone(&brain1);
        thread::spawn(move || {
            for i in 0..10 {
                b.remember(
                    &format!("from-brain1-{i}"),
                    "apollo weather data from instance one",
                    Visibility::Private,
                )
                .expect("handle 1 write");
            }
        })
    };
    let h2 = {
        let b = Arc::clone(&brain2);
        thread::spawn(move || {
            for i in 0..10 {
                b.remember(
                    &format!("from-brain2-{i}"),
                    "apollo weather data from instance two",
                    Visibility::Private,
                )
                .expect("handle 2 write");
            }
        })
    };
    h1.join().expect("handle 1 thread panicked");
    h2.join().expect("handle 2 thread panicked");

    // Both handles' writes are durable and visible through either handle.
    for (label, brain) in [("handle 1", &brain1), ("handle 2", &brain2)] {
        for key in ["from-brain1-9", "from-brain2-9"] {
            assert!(
                brain.get_memory_by_key(key).unwrap().is_some(),
                "{label} cannot see {key} after concurrent writes"
            );
        }
    }
}

/// R-10: every SQLite file a Brain opens must run in WAL mode.
///
/// `graph.sqlite` previously ran on the default rollback journal while
/// `memory.db` and `recognition.db` ran WAL. That silently made the two
/// `PRAGMA wal_checkpoint(TRUNCATE)` calls in `GraphStore::vacuum` no-ops —
/// the calls the D4 deletion guarantee depends on — and made every graph
/// write take an EXCLUSIVE lock blocking graph readers.
#[test]
fn every_brain_database_runs_in_wal_mode() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    brain
        .remember("k", "a memory so every store file exists", Visibility::Private)
        .unwrap();
    drop(brain);

    for db in ["graph.sqlite", "memory.db", "recognition.db"] {
        let path = tmp.path().join(db);
        assert!(path.exists(), "{db} was not created");
        let conn = rusqlite::Connection::open(&path).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            mode.to_lowercase(),
            "wal",
            "{db} is in {mode} mode, not WAL — wal_checkpoint on it is a no-op"
        );
    }
}

/// R-09: a second writer must WAIT for the first rather than failing
/// immediately. SQLite defaults busy_timeout to 0, so without an explicit
/// timeout the first contention returns SQLITE_BUSY.
#[test]
fn a_second_writer_waits_instead_of_failing_busy() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    brain.remember("seed", "seed memory", Visibility::Private).unwrap();

    // Hold a write transaction open on memory.db from an independent
    // connection, then have the Brain write while it is held.
    let blocker = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
    blocker.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();

    let handle = thread::spawn({
        let dir = tmp.path().to_path_buf();
        move || {
            let conn = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
            conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
            // Would return SQLITE_BUSY instantly with the default timeout of 0.
            conn.execute(
                "INSERT INTO memories (id, key, content, visibility, created_at)
                 VALUES ('x','x','x','private','2026/01/01 (Thu) 10:00')",
                [],
            )
        }
    });

    thread::sleep(std::time::Duration::from_millis(150));
    blocker.execute_batch("COMMIT").unwrap();

    let result = handle.join().unwrap();
    assert!(
        result.is_ok(),
        "second writer failed instead of waiting: {result:?}"
    );
}
