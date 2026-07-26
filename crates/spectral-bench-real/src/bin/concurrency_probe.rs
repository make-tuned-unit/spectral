//! Does recall throughput scale with concurrent readers?
//!
//! Deterministic, $0, no LLM. `SqliteStore` serialises every operation through
//! one `Arc<Mutex<Connection>>`, including reads — but WAL permits concurrent
//! readers. This measures whether adding reader threads buys throughput or
//! whether the mutex caps it, which decides if a read-connection pool is worth
//! building for server workloads.

use std::sync::Arc;
use std::time::Instant;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, RecallTopKConfig};

const N: usize = 600;
const PER_THREAD: usize = 120;
const QUERIES: &[&str] = &[
    "deployment region halifax",
    "sprint retrospective open bugs",
    "on-call rotation checklist",
    "team notes for the release",
];

fn brain_config(dir: &std::path::Path) -> BrainConfig {
    BrainConfig {
        data_dir: dir.to_path_buf(),
        ontology_path: dir.join("ontology.toml"),
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
    }
}

fn main() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("ontology.toml"), "version = 1\n").unwrap();
    let brain = Brain::open(brain_config(tmp.path())).unwrap();
    for i in 0..N {
        brain
            .remember(
                &format!("bench:key:{i}"),
                &format!(
                    "Memory {i}: the deployment region is Halifax, sprint {} \
                     checklist, open bugs, retrospective notes for team {}.",
                    i % 12,
                    i % 5
                ),
                Visibility::Private,
            )
            .unwrap();
    }
    let brain = Arc::new(brain);

    println!("=== Concurrent recall throughput ({N} memories, {PER_THREAD} recalls/thread) ===\n");
    println!(
        "{:>8}  {:>12}  {:>14}  {:>10}  {:>9}",
        "threads", "wall ms", "recalls/sec", "speedup", "efficiency"
    );

    let mut base = 0.0f64;
    for &threads in &[1usize, 2, 4, 8] {
        let cfg = RecallTopKConfig::default();
        let start = Instant::now();
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let b = Arc::clone(&brain);
                let cfg = cfg.clone();
                std::thread::spawn(move || {
                    for i in 0..PER_THREAD {
                        let q = QUERIES[(i + t) % QUERIES.len()];
                        let _ = b.recall_topk_fts(q, &cfg, Visibility::Private);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let wall = start.elapsed().as_secs_f64();
        let total = (threads * PER_THREAD) as f64;
        let rps = total / wall;
        if threads == 1 {
            base = rps;
        }
        println!(
            "{:>8}  {:>12.1}  {:>14.0}  {:>9.2}x  {:>8.0}%",
            threads,
            wall * 1000.0,
            rps,
            rps / base,
            100.0 * (rps / base) / threads as f64
        );
    }
    println!(
        "\nPerfect scaling = speedup equal to thread count. Flat throughput means\n\
         the single connection mutex is the ceiling, and a read pool would lift it."
    );
}
