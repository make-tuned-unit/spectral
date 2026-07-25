//! Where does TACT's ~4.4 ms fixed cost per cascade recall go?
//!
//! Deterministic, $0, no LLM. TACT accounts for ~80% of cascade recall latency
//! and is 8x the cost of raw FTS for the same candidate job. This times its
//! stages against a real store so the expensive one is identified rather than
//! guessed at. Nothing here proposes removing a tier — the goal is to find
//! which one to make faster.

use std::time::Instant;

use spectral_ingest::ingest::{ingest_with, IngestConfig, IngestOpts};
use spectral_ingest::sqlite_store::SqliteStore;
use spectral_ingest::MemoryStore;

const N: usize = 800;
const REPS: usize = 40;
const QUERIES: &[&str] = &[
    "deployment region halifax",
    "sprint retrospective open bugs",
    "on-call rotation checklist",
    "team notes for the release",
    "what did we decide about staging",
];

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = SqliteStore::open(&tmp.path().join("memory.db")).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let cfg = IngestConfig::default();
    for i in 0..N {
        rt.block_on(ingest_with(
            &format!("{i:016x}"),
            &format!("bench:key:{i}"),
            &format!(
                "Memory number {i}: the deployment region is Halifax and the on-call \
                 rotation for sprint {} covers deploy checklist items, open bugs, and \
                 the retrospective notes for team {}.",
                i % 12,
                i % 5
            ),
            "note",
            0.0,
            "private",
            &cfg,
            &store,
            IngestOpts::default(),
        ))
        .unwrap();
    }

    let tcfg = spectral_tact::TactConfig::default();
    let (mut t_cls, mut t_wing, mut t_fts, mut t_search, mut t_full) =
        (vec![], vec![], vec![], vec![], vec![]);

    for _ in 0..REPS {
        for q in QUERIES {
            let s = Instant::now();
            let w = spectral_tact::classifier::detect_wing(q, &tcfg.wing_rules);
            let h = spectral_tact::classifier::detect_hall(q, &tcfg.hall_rules);
            t_cls.push(s.elapsed().as_secs_f64() * 1000.0);

            if let Some(ref ww) = w {
                let terms: Vec<String> = q.split_whitespace().map(|t| t.to_lowercase()).collect();
                let s = Instant::now();
                let _ = rt.block_on(store.wing_search(ww, &terms, 40));
                t_wing.push(s.elapsed().as_secs_f64() * 1000.0);
            }

            let words: Vec<String> = q
                .split_whitespace()
                .filter(|t| t.len() > 2)
                .map(|t| t.to_lowercase())
                .collect();
            let s = Instant::now();
            let _ = rt.block_on(store.fts_search(&words, 40));
            t_fts.push(s.elapsed().as_secs_f64() * 1000.0);

            let s = Instant::now();
            let _ = rt.block_on(spectral_tact::extractor::search(q, &w, &h, &tcfg, &store));
            t_search.push(s.elapsed().as_secs_f64() * 1000.0);

            let s = Instant::now();
            let _ = rt.block_on(spectral_tact::retrieve_memories(q, &tcfg, &store));
            t_full.push(s.elapsed().as_secs_f64() * 1000.0);
        }
    }

    // Cost of the clone `Brain::tact_retrieve_with_k` performs on every call.
    let mut t_clone = vec![];
    for _ in 0..(REPS * QUERIES.len()) {
        let s = Instant::now();
        let mut c = tcfg.clone();
        c.max_results = 40;
        std::hint::black_box(&c);
        t_clone.push(s.elapsed().as_secs_f64() * 1000.0);
    }

    println!("=== TACT tier cost breakdown (N={N} memories, release) ===\n");
    println!("TactConfig::clone (per call)  {:>8.4} ms", median(t_clone));
    println!("classify (wing+hall regex)   {:>8.4} ms", median(t_cls));
    println!("store.wing_search            {:>8.4} ms", median(t_wing));
    println!("store.fts_search             {:>8.4} ms", median(t_fts));
    println!("extractor::search (all tiers){:>8.4} ms", median(t_search));
    println!("retrieve_memories (full)     {:>8.4} ms", median(t_full));
    println!(
        "\nwing classified: {}   (tier-2 runs only when a wing is detected)",
        spectral_tact::classifier::detect_wing(QUERIES[0], &tcfg.wing_rules).is_some()
    );
}
