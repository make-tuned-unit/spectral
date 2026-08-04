//! Per-stage profile of Spectral's own ingest work.
//!
//! `ingest-gap-decomposition-2026-08-03.md` found that **73%** of the remaining
//! ingest gap to MinHash+BM25 is Spectral's per-event work, not SQLite — and
//! that it had never been profiled at stage granularity. This does that.
//!
//! Stages mirror `ingest::ingest_with` in order. Each is timed by replaying the
//! same call the real path makes, against a store warmed with the same corpus.
//!
//! Deterministic, $0. Release, warm, warm-up discarded, two runs.
//!
//! `cargo run -p spectral-bench-real --release --bin ingest_stage_profile`

use std::time::Instant;

use spectral_ingest::ingest::{ingest_with, IngestConfig, IngestOpts};
use spectral_ingest::sqlite_store::SqliteStore;
use spectral_ingest::{classifier, signal, MemoryStore};

const N: usize = 1500;
const WARMUP: usize = 200;
const EPISODE_GAP_MINUTES: i64 = 30;

fn content(i: usize) -> String {
    format!(
        "the deploy pipeline was rolled back after the platform team review \
         (record {i}, ref {:04x}) covering staging checklist items and open bugs",
        i.wrapping_mul(2654435761) & 0xffff
    )
}

fn ms(d: std::time::Duration, n: usize) -> f64 {
    d.as_secs_f64() * 1000.0 / n as f64
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("WARNING: debug build — use --release.\n");
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    let cfg = IngestConfig {
        fingerprints: false,
        ..IngestConfig::default()
    };

    for pass in 1..=2 {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = SqliteStore::open(&tmp.path().join("memory.db")).unwrap();

        // Warm the store so episode lookup has realistic work to do.
        for i in 0..WARMUP {
            rt.block_on(ingest_with(
                &format!("{i:016x}"),
                &format!("w-{i}"),
                &content(i),
                "note",
                0.0,
                "private",
                &cfg,
                &store,
                IngestOpts::default(),
            ))
            .unwrap();
        }

        // ── full path, for the denominator ──
        let t = Instant::now();
        for i in 0..N {
            rt.block_on(ingest_with(
                &format!("{:016x}", i + 900_000),
                &format!("f-{i}"),
                &content(i),
                "note",
                0.0,
                "private",
                &cfg,
                &store,
                IngestOpts::default(),
            ))
            .unwrap();
        }
        let full = ms(t.elapsed(), N);

        // ── stage: classify wing ──
        let t = Instant::now();
        let mut sink = 0usize;
        for i in 0..N {
            let w =
                classifier::classify_wing(&format!("f-{i}"), &content(i), "note", &cfg.wing_rules);
            sink += w.len();
        }
        let s_wing = ms(t.elapsed(), N);

        // ── stage: classify hall ──
        let t = Instant::now();
        for i in 0..N {
            sink += classifier::classify_hall(&content(i), &cfg.hall_rules).len();
        }
        let s_hall = ms(t.elapsed(), N);

        // ── stage: signal score ──
        let t = Instant::now();
        for i in 0..N {
            sink += (signal::score_memory(&content(i), "fact") * 1000.0) as usize;
        }
        let s_signal = ms(t.elapsed(), N);
        std::hint::black_box(sink);

        // ── stage: episode lookup (one store read per ingest) ──
        let since = (chrono::Utc::now() - chrono::Duration::minutes(EPISODE_GAP_MINUTES))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let t = Instant::now();
        for _ in 0..N {
            let _ = rt
                .block_on(store.find_recent_episode("general", &since))
                .unwrap();
        }
        let s_ep_read = ms(t.elapsed(), N);

        // ── stage: episode write (one store write per ingest) ──
        let ep = rt
            .block_on(store.find_recent_episode("general", "1970-01-01 00:00:00"))
            .unwrap();
        let s_ep_write = match ep {
            Some(mut e) => {
                let t = Instant::now();
                for _ in 0..N {
                    e.memory_count += 1;
                    rt.block_on(store.write_episode(&e)).unwrap();
                }
                ms(t.elapsed(), N)
            }
            None => f64::NAN,
        };

        let accounted = s_wing + s_hall + s_signal + s_ep_read + s_ep_write;
        println!("--- run {pass} (N={N}, fingerprints OFF) ---");
        println!("{:<34} {:>10} {:>9}", "stage", "ms/event", "% of full");
        let row = |name: &str, v: f64| {
            println!("{name:<34} {v:>10.4} {:>8.1}%", 100.0 * v / full);
        };
        row("classify wing", s_wing);
        row("classify hall", s_hall);
        row("signal score", s_signal);
        row("episode lookup (store read)", s_ep_read);
        row("episode write (store write)", s_ep_write);
        println!("{:-<55}", "");
        row("accounted", accounted);
        row("memory write + unaccounted", full - accounted);
        println!("{:<34} {full:>10.4} {:>8.1}%\n", "FULL ingest_with", 100.0);
    }
}
