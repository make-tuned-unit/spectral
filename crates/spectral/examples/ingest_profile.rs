//! $0 ingest throughput profiler — attributes `Brain::remember` cost to layers.
//!
//! Phase 0 measured Spectral at 43 ev/s against MinHash+BM25's 21,800 ev/s
//! (`docs/internal/PHASE0_RESULTS.md`) — a ~500× systems loss — and the
//! fingerprint-fanout work left `remember` still growing 2.8× over 800 writes,
//! attributing the residue to recognition enrollment by inspection rather than
//! measurement (`docs/internal/fingerprint-fanout-cap-2026-07-25.md`).
//!
//! This binary measures the split directly, with no API spend:
//!
//! * **store floor** — `ingest::ingest_with` against a bare `SqliteStore`:
//!   classify + score + fingerprint + write. No graph Brain.
//! * **full remember** — `Brain::remember`, which adds session association,
//!   declarative density, provenance signing, recurrence feedback, and
//!   recognition enrollment, each as its own runtime round-trip.
//! * **growth** — per-quartile rate, to expose superlinear cost as the corpus
//!   grows (the 2.8× effect) rather than reporting one blended average.
//!
//! Run: `cargo run -p spectral --release --example ingest_profile [N]`
//!
//! # Measure WARM, and more than once
//!
//! Release mode matters — debug numbers are not comparable to the Phase 0
//! figures, which were release. **So does machine state.** The first published
//! numbers from this tool were taken immediately after a cold ~8-minute
//! compile and were inflated ~2.8x by disk contention and thermal state
//! (2.736 ms/write warm was reported as 7.66 ms). They were a single run with
//! no stability check.
//!
//! This binary now discards a warm-up pass before timing, but that does not
//! substitute for judgement: **run it at least twice, on an otherwise idle
//! machine, and treat runs that disagree by more than ~15% as unusable.**
//!
//! Reference figures, warm, n=400 release, on an M-series mac at commit
//! 3005186 (see `docs/internal/ingest-cost-profile-2026-07-31.md`):
//!
//! | layer | ms/write | ev/s |
//! |---|---|---|
//! | store, no fingerprints | ~0.20 | ~5,000 |
//! | store floor | ~1.1-1.3 | ~800-920 |
//! | full `Brain::remember` | ~2.3-2.7 | ~365-430 |
//!
//! Ratios travel between machines; absolute numbers do not.

use std::time::{Duration, Instant};

use spectral::{Brain, Visibility};
use spectral_ingest::ingest::{ingest_with, IngestConfig, IngestOpts};
use spectral_ingest::sqlite_store::SqliteStore;
use tempfile::TempDir;

/// Synthetic memory with enough lexical variety that classification,
/// fingerprinting, and shingle enrollment all do realistic work.
fn content_for(i: usize) -> String {
    const SUBJECTS: [&str; 8] = [
        "the deploy pipeline",
        "the billing reconciliation job",
        "the onboarding checklist",
        "the incident retrospective",
        "the capacity forecast",
        "the vendor contract review",
        "the schema migration",
        "the access audit",
    ];
    const VERBS: [&str; 6] = [
        "was updated by",
        "was blocked on",
        "was approved by",
        "was rolled back after",
        "was escalated to",
        "was deferred pending",
    ];
    const OBJECTS: [&str; 7] = [
        "the platform team on Tuesday",
        "a stale credential in staging",
        "the finance lead before quarter close",
        "an unexpected lock timeout",
        "the on-call engineer overnight",
        "a missing upstream approval",
        "the compliance sign-off window",
    ];
    format!(
        "{} {} {} (record {i}, ref {:04x})",
        SUBJECTS[i % SUBJECTS.len()],
        VERBS[(i / 8) % VERBS.len()],
        OBJECTS[(i / 3) % OBJECTS.len()],
        i.wrapping_mul(2654435761) & 0xffff,
    )
}

fn rate(n: usize, elapsed: Duration) -> f64 {
    n as f64 / elapsed.as_secs_f64()
}

/// Writes discarded before timing starts, to absorb first-touch costs (page
/// cache, WAL creation, allocator warm-up) that otherwise land entirely in Q1
/// and inflate the whole run. Keyed into the same id space, so the timed
/// writes still operate against a non-empty store.
const WARMUP_WRITES: usize = 25;

/// Time `n` writes, returning total elapsed and per-quartile elapsed.
///
/// Runs — and discards — [`WARMUP_WRITES`] iterations first. This does not make
/// the measurement immune to a loaded machine: see the module docs.
fn timed_quartiles<F: FnMut(usize)>(n: usize, mut write: F) -> (Duration, Vec<Duration>) {
    for i in 0..WARMUP_WRITES {
        write(usize::MAX - i);
    }
    let q = (n / 4).max(1);
    let mut quartiles = Vec::new();
    let overall = Instant::now();
    let mut mark = Instant::now();
    for i in 0..n {
        write(i);
        if (i + 1) % q == 0 && quartiles.len() < 4 {
            quartiles.push(mark.elapsed());
            mark = Instant::now();
        }
    }
    (overall.elapsed(), quartiles)
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(400);

    if cfg!(debug_assertions) {
        eprintln!(
            "WARNING: debug build. Numbers are NOT comparable to the release-mode \
             Phase 0 baselines. Re-run with --release.\n"
        );
    }

    println!("ingest profile — n={n}\n");

    // ── Layer 1: store floor (no graph Brain) ──────────────────────────
    let store_tmp = TempDir::new().unwrap();
    let store = SqliteStore::open(&store_tmp.path().join("memory.db")).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let config = IngestConfig::default();

    let (store_total, store_q) = timed_quartiles(n, |i| {
        rt.block_on(ingest_with(
            &format!("floor-{i}"),
            &format!("floor-{i}"),
            &content_for(i),
            "core",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts::default(),
        ))
        .unwrap();
    });

    // ── Layer 1b: store floor with fingerprint generation suppressed ───
    // `signal_threshold` gates fingerprint generation; a threshold above any
    // achievable score turns it off, isolating the constellation-fingerprint
    // cost from the rest of ingest (classify + score + FTS + row write).
    let nofp_tmp = TempDir::new().unwrap();
    let nofp_store = SqliteStore::open(&nofp_tmp.path().join("memory.db")).unwrap();
    let nofp_config = IngestConfig {
        signal_threshold: 2.0,
        ..IngestConfig::default()
    };
    let (nofp_total, nofp_q) = timed_quartiles(n, |i| {
        rt.block_on(ingest_with(
            &format!("nofp-{i}"),
            &format!("nofp-{i}"),
            &content_for(i),
            "core",
            0.0,
            "private",
            &nofp_config,
            &nofp_store,
            IngestOpts::default(),
        ))
        .unwrap();
    });

    // ── Layer 2: full Brain::remember ──────────────────────────────────
    let brain_tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_tmp.path()).unwrap();
    let (brain_total, brain_q) = timed_quartiles(n, |i| {
        brain
            .remember(&format!("full-{i}"), &content_for(i), Visibility::Private)
            .unwrap();
    });

    // ── Report ─────────────────────────────────────────────────────────
    let store_rate = rate(n, store_total);
    let brain_rate = rate(n, brain_total);
    let per_write_store = store_total.as_secs_f64() * 1000.0 / n as f64;
    let per_write_brain = brain_total.as_secs_f64() * 1000.0 / n as f64;

    println!("{:<28} {:>12} {:>14}", "layer", "ev/s", "ms/write");
    println!("{:-<56}", "");
    let per_write_nofp = nofp_total.as_secs_f64() * 1000.0 / n as f64;
    println!(
        "{:<28} {:>12.0} {:>14.3}",
        "store, no fingerprints",
        rate(n, nofp_total),
        per_write_nofp
    );
    println!(
        "{:<28} {:>12.0} {:>14.3}",
        "store floor (ingest_with)", store_rate, per_write_store
    );
    println!(
        "{:<28} {:>12} {:>14.3}",
        "  └ fingerprint cost",
        "—",
        per_write_store - per_write_nofp
    );
    println!(
        "{:<28} {:>12.0} {:>14.3}",
        "full Brain::remember", brain_rate, per_write_brain
    );
    println!(
        "{:<28} {:>12} {:>14.3}",
        "graph-side overhead",
        "—",
        per_write_brain - per_write_store
    );
    println!(
        "\ngraph-side share of a write: {:.1}%",
        100.0 * (per_write_brain - per_write_store) / per_write_brain
    );
    println!(
        "Brain is {:.1}× slower than the store floor.",
        per_write_brain / per_write_store
    );

    println!("\ngrowth by quartile (ms/write — flat means cost is O(1) in corpus size)");
    println!(
        "{:<28} {:>8} {:>8} {:>8} {:>8} {:>10}",
        "layer", "Q1", "Q2", "Q3", "Q4", "Q4/Q1"
    );
    println!("{:-<74}", "");
    let q = (n / 4).max(1);
    for (label, qs) in [
        ("store, no fingerprints", &nofp_q),
        ("store floor", &store_q),
        ("Brain::remember", &brain_q),
    ] {
        if qs.len() == 4 {
            let ms: Vec<f64> = qs
                .iter()
                .map(|d| d.as_secs_f64() * 1000.0 / q as f64)
                .collect();
            println!(
                "{:<28} {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>10.2}",
                label,
                ms[0],
                ms[1],
                ms[2],
                ms[3],
                ms[3] / ms[0]
            );
        }
    }

    // ── Storage footprint ──────────────────────────────────────────────
    println!("\nstorage (KB/event, including WAL)");
    for (label, dir) in [
        ("store, no fingerprints", nofp_tmp.path()),
        ("store floor", store_tmp.path()),
        ("Brain", brain_tmp.path()),
    ] {
        let bytes: u64 = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum();
        println!("  {:<26} {:>8.1}", label, bytes as f64 / 1024.0 / n as f64);
    }
}
