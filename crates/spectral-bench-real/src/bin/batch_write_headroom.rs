//! How much of Spectral's remaining ingest gap is per-event transaction commit?
//!
//! `SqliteStore::write` opens `conn.transaction()` per memory, so every ingest
//! is its own WAL commit. This isolates that cost: identical rows and identical
//! FTS5 trigger work, written one-transaction-per-row vs all-in-one, against
//! the real `memories` schema.
//!
//! Deterministic, $0. Release, warm, two runs.
//!
//! `cargo run -p spectral-bench-real --release --bin batch_write_headroom`

use std::time::Instant;

use rusqlite::Connection;

const N: usize = 3000;

fn schema(conn: &Connection) {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous  = NORMAL;
         CREATE TABLE memories (
            id TEXT PRIMARY KEY, key TEXT NOT NULL UNIQUE, content TEXT NOT NULL,
            category TEXT NOT NULL DEFAULT 'core', wing TEXT, hall TEXT,
            signal_score REAL DEFAULT 0.5, visibility TEXT NOT NULL DEFAULT 'private',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            source TEXT, device_id BLOB, confidence REAL NOT NULL DEFAULT 1.0,
            description TEXT);
         CREATE VIRTUAL TABLE memories_fts USING fts5(
            key, content, description, content=memories, content_rowid=rowid,
            tokenize = 'porter unicode61');
         CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, key, content, description)
            VALUES (new.rowid, new.key, new.content, COALESCE(new.description,''));
         END;",
    )
    .unwrap();
}

fn content(i: usize) -> String {
    format!(
        "the deploy pipeline was rolled back after the platform team review \
         (record {i}, ref {:04x}) covering staging checklist items and open bugs",
        i.wrapping_mul(2654435761) & 0xffff
    )
}

/// The seven secondary indexes the real `memories` table carries.
fn secondary_indexes(conn: &Connection) {
    conn.execute_batch(
        "CREATE INDEX idx_memories_key ON memories(key);
         CREATE INDEX idx_memories_wing ON memories(wing);
         CREATE INDEX idx_memories_signal ON memories(signal_score);
         CREATE INDEX idx_memories_wing_recency
            ON memories(wing, datetime(created_at) DESC, id DESC);
         CREATE INDEX idx_memories_episode_id ON memories(episode_id);
         CREATE INDEX idx_memories_content_hash ON memories(content_hash);
         CREATE INDEX idx_memories_source_brain_id ON memories(source_brain_id);",
    )
    .unwrap();
}

/// The five indexes that remain after dropping the two redundant ones:
/// `idx_memories_key` (duplicates the UNIQUE constraint's implicit index) and
/// `idx_memories_wing` (a prefix of `idx_memories_wing_recency`).
fn lean_indexes(conn: &Connection) {
    conn.execute_batch(
        "CREATE INDEX idx_memories_signal ON memories(signal_score);
         CREATE INDEX idx_memories_wing_recency
            ON memories(wing, datetime(created_at) DESC, id DESC);
         CREATE INDEX idx_memories_episode_id ON memories(episode_id);
         CREATE INDEX idx_memories_content_hash ON memories(content_hash);
         CREATE INDEX idx_memories_source_brain_id ON memories(source_brain_id);",
    )
    .unwrap();
}

fn per_row_txn_lean(path: &std::path::Path) -> f64 {
    let conn = Connection::open(path).unwrap();
    schema(&conn);
    conn.execute_batch(
        "ALTER TABLE memories ADD COLUMN episode_id TEXT;
         ALTER TABLE memories ADD COLUMN content_hash TEXT;
         ALTER TABLE memories ADD COLUMN source_brain_id BLOB;",
    )
    .unwrap();
    lean_indexes(&conn);
    let start = Instant::now();
    for i in 0..N {
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO memories (id, key, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![format!("{i:016x}"), format!("m-{i}"), content(i)],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    start.elapsed().as_secs_f64()
}

fn per_row_txn_indexed(path: &std::path::Path) -> f64 {
    let conn = Connection::open(path).unwrap();
    schema(&conn);
    conn.execute_batch(
        "ALTER TABLE memories ADD COLUMN episode_id TEXT;
         ALTER TABLE memories ADD COLUMN content_hash TEXT;
         ALTER TABLE memories ADD COLUMN source_brain_id BLOB;",
    )
    .unwrap();
    secondary_indexes(&conn);
    let start = Instant::now();
    for i in 0..N {
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO memories (id, key, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![format!("{i:016x}"), format!("m-{i}"), content(i)],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    start.elapsed().as_secs_f64()
}

fn per_row_txn(path: &std::path::Path) -> f64 {
    let conn = Connection::open(path).unwrap();
    schema(&conn);
    let start = Instant::now();
    for i in 0..N {
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO memories (id, key, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![format!("{i:016x}"), format!("m-{i}"), content(i)],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    start.elapsed().as_secs_f64()
}

fn one_txn(path: &std::path::Path) -> f64 {
    let conn = Connection::open(path).unwrap();
    schema(&conn);
    let start = Instant::now();
    let tx = conn.unchecked_transaction().unwrap();
    {
        let mut stmt = tx
            .prepare("INSERT INTO memories (id, key, content) VALUES (?1, ?2, ?3)")
            .unwrap();
        for i in 0..N {
            stmt.execute(rusqlite::params![
                format!("{i:016x}"),
                format!("m-{i}"),
                content(i)
            ])
            .unwrap();
        }
    }
    tx.commit().unwrap();
    start.elapsed().as_secs_f64()
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("WARNING: debug build — use --release.\n");
    }
    println!("per-event transaction overhead — N={N}, WAL, synchronous=NORMAL\n");
    println!(
        "{:<28} {:>12} {:>14} {:>10}",
        "mode", "ms total", "ms/event", "ev/s"
    );

    for pass in 1..=2 {
        let d1 = tempfile::TempDir::new().unwrap();
        let d2 = tempfile::TempDir::new().unwrap();
        let d3 = tempfile::TempDir::new().unwrap();
        let a = per_row_txn(&d1.path().join("a.db"));
        let b = one_txn(&d2.path().join("b.db"));
        let c = per_row_txn_indexed(&d3.path().join("c.db"));
        let d4 = tempfile::TempDir::new().unwrap();
        let d = per_row_txn_lean(&d4.path().join("d.db"));
        println!(
            "run {pass}: one txn per row      {:>12.1} {:>14.4} {:>10.0}",
            a * 1000.0,
            a * 1000.0 / N as f64,
            N as f64 / a
        );
        println!(
            "run {pass}: single batched txn   {:>12.1} {:>14.4} {:>10.0}   speedup {:.1}x",
            b * 1000.0,
            b * 1000.0 / N as f64,
            N as f64 / b,
            a / b
        );
        println!(
            "run {pass}: per row + 7 indexes  {:>12.1} {:>14.4} {:>10.0}   {:.1}x the 2-index cost",
            c * 1000.0,
            c * 1000.0 / N as f64,
            N as f64 / c,
            c / a
        );
        println!(
            "run {pass}: per row + 5 (lean)     {:>12.1} {:>14.4} {:>10.0}   {:.0}% faster than 7",
            d * 1000.0,
            d * 1000.0 / N as f64,
            N as f64 / d,
            100.0 * (c - d) / c
        );
    }
}
