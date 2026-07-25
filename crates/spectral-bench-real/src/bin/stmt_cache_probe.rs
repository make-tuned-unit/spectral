//! Is `conn.prepare()` on the hot read path worth converting to
//! `prepare_cached()`? Deterministic, $0, no LLM.
//!
//! The cascade/topk FTS query is rebuilt with `format!` and recompiled by
//! SQLite on every single recall. This measures the compile cost in isolation
//! against the same query served from the statement cache, so the decision is
//! made on a number rather than on intuition.

use std::time::Instant;

const REPS: usize = 2000;

fn main() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db = tmp.path().join("probe.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE memories (
            id TEXT PRIMARY KEY, key TEXT UNIQUE, content TEXT, wing TEXT,
            hall TEXT, signal_score REAL, visibility TEXT, created_at TEXT,
            description TEXT);
         CREATE TABLE consolidation_edges (
            source_key TEXT NOT NULL, target_key TEXT NOT NULL,
            PRIMARY KEY (source_key, target_key));
         CREATE VIRTUAL TABLE memories_fts USING fts5(
            key, content, description, content='memories', content_rowid='rowid',
            tokenize='porter unicode61');",
    )
    .unwrap();

    for i in 0..2000 {
        conn.execute(
            "INSERT INTO memories (id, key, content, wing, hall, signal_score, visibility, created_at)
             VALUES (?1, ?2, ?3, 'general', 'fact', 0.8, 'private', datetime('now'))",
            rusqlite::params![
                format!("{i:016x}"),
                format!("k{i}"),
                format!("Memory {i}: deployment region Halifax sprint retrospective open bugs")
            ],
        )
        .unwrap();
    }
    conn.execute_batch("INSERT INTO memories_fts(memories_fts) VALUES('rebuild');")
        .unwrap();

    // The real shape from sqlite_store: FTS join + NOT IN subquery + bm25 order.
    let build_sql = || {
        format!(
            "SELECT m.{cols}
             FROM memories_fts fts
             JOIN memories m ON m.rowid = fts.rowid
             WHERE memories_fts MATCH ?1
               AND m.key NOT IN (SELECT source_key FROM consolidation_edges)
             ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5) LIMIT ?2",
            cols = "id, m.key, m.content, m.wing, m.hall, m.signal_score, m.visibility"
        )
    };

    // `prepare` and `prepare_cached` return different types, so the two
    // arms are separate loops rather than a branch inside one.
    macro_rules! drain {
        ($stmt:expr) => {{
            let rows = $stmt
                .query_map(rusqlite::params!["deployment", 40i64], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap();
            let mut n = 0;
            for r in rows {
                r.unwrap();
                n += 1;
                if n >= 40 {
                    break;
                }
            }
        }};
    }
    let run = |cached: bool| -> f64 {
        let t = Instant::now();
        for _ in 0..REPS {
            let sql = build_sql();
            if cached {
                let mut stmt = conn.prepare_cached(&sql).unwrap();
                drain!(stmt);
            } else {
                let mut stmt = conn.prepare(&sql).unwrap();
                drain!(stmt);
            }
        }
        t.elapsed().as_secs_f64() * 1000.0 / REPS as f64
    };

    // Warm both paths before timing.
    let _ = run(false);
    let _ = run(true);

    let plain = run(false);
    let cached = run(true);

    // Compile cost alone, no row fetching.
    let compile = |cached: bool| -> f64 {
        let t = Instant::now();
        for _ in 0..REPS {
            let sql = build_sql();
            if cached {
                let _ = conn.prepare_cached(&sql).unwrap();
            } else {
                let _ = conn.prepare(&sql).unwrap();
            }
        }
        t.elapsed().as_secs_f64() * 1000.0 / REPS as f64
    };
    let cplain = compile(false);
    let ccached = compile(true);

    println!("=== Statement cache probe (2000 memories, {REPS} reps) ===\n");
    println!("full query   prepare()        {plain:.4} ms/call");
    println!("full query   prepare_cached() {cached:.4} ms/call");
    println!(
        "             saving           {:.4} ms/call ({:.1}%)\n",
        plain - cached,
        100.0 * (plain - cached) / plain
    );
    println!("compile only prepare()        {cplain:.4} ms/call");
    println!("compile only prepare_cached() {ccached:.4} ms/call");
    println!(
        "             saving           {:.4} ms/call",
        cplain - ccached
    );
    println!(
        "\nContext: a cascade recall measured ~5.6 ms end to end. A {:.4} ms saving\nper prepared statement is {:.2}% of that per statement.",
        cplain - ccached,
        100.0 * (cplain - ccached) / 5.6
    );
}
