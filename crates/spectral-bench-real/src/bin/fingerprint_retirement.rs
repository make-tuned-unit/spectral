//! What does retiring constellation fingerprints actually buy — and cost?
//!
//! Measures write time and on-disk bytes with fingerprints ON vs OFF over the
//! same synthetic corpus, warm, both arms in one process.
//!
//! Method rules from `docs/internal/ingest-cost-profile-2026-07-31.md`: release,
//! warm, discard a warm-up pass, report more than one run.
//!
//! `cargo run -p spectral-bench-real --release --bin fingerprint_retirement`

use std::time::Instant;

use spectral_ingest::ingest::{ingest_with, IngestConfig, IngestOpts};
use spectral_ingest::sqlite_store::SqliteStore;

const N: usize = 600;
const WARMUP: usize = 50;

fn content_for(i: usize) -> String {
    const S: [&str; 6] = [
        "the deploy pipeline",
        "the billing reconciliation job",
        "the incident retrospective",
        "the capacity forecast",
        "the schema migration",
        "the access audit",
    ];
    const V: [&str; 5] = [
        "was updated by",
        "was blocked on",
        "was rolled back after",
        "was escalated to",
        "was deferred pending",
    ];
    format!(
        "{} {} the platform team (record {i}, ref {:04x})",
        S[i % S.len()],
        V[(i / 6) % V.len()],
        i.wrapping_mul(2654435761) & 0xffff,
    )
}

fn dir_bytes(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

fn run(label: &str, fingerprints: bool, rt: &tokio::runtime::Runtime) -> (f64, u64, i64) {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = SqliteStore::open(&tmp.path().join("memory.db")).unwrap();
    let config = IngestConfig {
        fingerprints,
        ..IngestConfig::default()
    };

    for i in 0..WARMUP {
        rt.block_on(ingest_with(
            &format!("{i:016x}"),
            &format!("w-{i}"),
            &content_for(i),
            "note",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts::default(),
        ))
        .unwrap();
    }

    let start = Instant::now();
    for i in 0..N {
        rt.block_on(ingest_with(
            &format!("{:016x}", i + 100_000),
            &format!("m-{i}"),
            &content_for(i),
            "note",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts::default(),
        ))
        .unwrap();
    }
    let ms_per_write = start.elapsed().as_secs_f64() * 1000.0 / N as f64;

    drop(store);
    let db = tmp.path().join("memory.db");
    let fp_rows: i64 = rusqlite::Connection::open(&db)
        .and_then(|c| {
            c.query_row("SELECT COUNT(*) FROM constellation_fingerprints", [], |r| {
                r.get(0)
            })
        })
        .unwrap_or(0);
    let bytes = dir_bytes(tmp.path());
    println!(
        "{label:28} {ms_per_write:>8.3} ms/write   {:>9.1} KB/event   fp rows {fp_rows}",
        bytes as f64 / 1024.0 / (N + WARMUP) as f64
    );
    (ms_per_write, bytes, fp_rows)
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("WARNING: debug build — use --release.\n");
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    println!("constellation fingerprint retirement — N={N}, warm, release\n");

    for pass in 1..=2 {
        println!("--- run {pass} ---");
        let (on_ms, on_bytes, on_rows) = run("fingerprints ON (default)", true, &rt);
        let (off_ms, off_bytes, off_rows) = run("fingerprints OFF", false, &rt);
        println!(
            "  speedup {:.2}x   storage {:.2}x smaller   rows {on_rows} -> {off_rows}\n",
            on_ms / off_ms,
            on_bytes as f64 / off_bytes.max(1) as f64
        );
    }
}
