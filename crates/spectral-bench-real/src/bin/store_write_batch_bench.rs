//! R7 measurement: `SqliteStore::write` loop vs `write_batch` — the actual
//! shipped API, on disk (commits must hit storage; in-memory would flatter
//! the batch).
//!
//! Register row R7 measured per-event commit at 21% of ingest cost and raw
//! batched SQLite at 60,489 ev/s vs 22,688 for MinHash+BM25. This bin closes
//! the loop on the API itself. Deterministic, $0. Release, warm, two runs
//! internal.
//!
//! `cargo run -p spectral-bench-real --release --bin store_write_batch_bench`

use std::time::Instant;

use spectral_ingest::sqlite_store::SqliteStore;
use spectral_ingest::{Fingerprint, Memory, MemoryStore};

const N: usize = 3000;

fn mem(i: usize, run: &str) -> Memory {
    Memory {
        id: format!("{run}-id-{i:06}"),
        key: format!("{run}-key-{i:06}"),
        content: format!(
            "the deploy pipeline was rolled back after the platform team review \
             (record {i}, ref {:04x}) covering staging checklist items and open bugs",
            i.wrapping_mul(2654435761) & 0xffff
        ),
        wing: Some("general".to_string()),
        hall: Some("fact".to_string()),
        signal_score: 0.7,
        visibility: "private".to_string(),
        source: None,
        device_id: None,
        confidence: 1.0,
        created_at: None,
        last_reinforced_at: None,
        episode_id: None,
        compaction_tier: None,
        declarative_density: None,
        description: None,
        description_generated_at: None,
        content_hash: None,
        source_brain_id: None,
        signature: None,
    }
}

fn items(run: &str) -> Vec<(Memory, Vec<Fingerprint>)> {
    (0..N).map(|i| (mem(i, run), Vec::new())).collect()
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    for pass in 1..=2 {
        let dir = tempfile::TempDir::new().unwrap();

        let seq_store = SqliteStore::open(&dir.path().join("seq.db")).unwrap();
        let seq_items = items("seq");
        let t = Instant::now();
        rt.block_on(async {
            for (m, f) in &seq_items {
                seq_store.write(m, f).await.unwrap();
            }
        });
        let seq_s = t.elapsed().as_secs_f64();

        let bat_store = SqliteStore::open(&dir.path().join("bat.db")).unwrap();
        let bat_items = items("bat");
        let t = Instant::now();
        let outcomes = rt.block_on(bat_store.write_batch(&bat_items)).unwrap();
        let bat_s = t.elapsed().as_secs_f64();
        assert_eq!(outcomes.len(), N);

        println!(
            "pass {pass}: sequential {:.0} ev/s ({seq_s:.2}s)   write_batch {:.0} ev/s ({bat_s:.2}s)   speedup {:.2}x",
            N as f64 / seq_s,
            N as f64 / bat_s,
            seq_s / bat_s
        );
    }
}
