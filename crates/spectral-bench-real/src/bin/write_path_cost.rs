//! Write-path cost breakdown: where does `Brain::remember()` time actually go?
//!
//! Deterministic, $0, no LLM. Measures the per-memory cost of the public write
//! path against two floors:
//!   - `MemoryStore::write` alone (primary row + fingerprints, one transaction)
//!   - N raw INSERTs inside ONE transaction (the batching floor)
//!
//! The gap between them bounds what a batch-write API could recover.

use std::time::Instant;

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};
use spectral_ingest::sqlite_store::SqliteStore;
use spectral_ingest::{Memory, MemoryStore};

const N: usize = 800;
const BUCKET: usize = 100;

fn corpus(n: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| {
            (
                format!("bench:key:{i}"),
                format!(
                    "Memory number {i}: the deployment region is Halifax and the \
                     on-call rotation for sprint {} covers deploy checklist items, \
                     open bugs, and the retrospective notes for team {}.",
                    i % 12,
                    i % 5
                ),
            )
        })
        .collect()
}

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

fn open_brain_at(dir: &std::path::Path) -> Brain {
    std::fs::write(dir.join("ontology.toml"), "version = 1\n").unwrap();
    Brain::open(brain_config(dir)).unwrap()
}

fn main() {
    let data = corpus(N);

    // ── A. Public path: Brain::remember ────────────────────────────────
    let tmp_a = tempfile::TempDir::new().unwrap();
    let mut brain = open_brain_at(tmp_a.path());
    // Force legacy unbounded pairing so arm A is the "before" regardless of
    // what the library default currently is.
    brain.set_max_fingerprint_peers(None);
    let mut buckets: Vec<std::time::Duration> = Vec::new();
    let mut t = Instant::now();
    for (i, (k, c)) in data.iter().enumerate() {
        brain.remember(k, c, Visibility::Private).unwrap();
        if (i + 1) % BUCKET == 0 {
            buckets.push(t.elapsed());
            t = Instant::now();
        }
    }
    let public_path: std::time::Duration = buckets.iter().sum();

    // ── B. MemoryStore::write alone (no derived writes) ────────────────
    let tmp_b = tempfile::TempDir::new().unwrap();
    let store_b = SqliteStore::open(&tmp_b.path().join("memory.db")).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let t = Instant::now();
    for (i, (k, c)) in data.iter().enumerate() {
        let m = Memory {
            id: format!("{i:016x}"),
            key: k.clone(),
            content: c.clone(),
            wing: Some("general".into()),
            hall: None,
            signal_score: 0.5,
            visibility: "private".into(),
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
        };
        rt.block_on(store_b.write(&m, &[])).unwrap();
    }
    let store_write_only = t.elapsed();

    // ── C. ingest_with only (classify + episode + fingerprints + write) ─
    let tmp_c = tempfile::TempDir::new().unwrap();
    let store_c = SqliteStore::open(&tmp_c.path().join("memory.db")).unwrap();
    let mut ing_buckets: Vec<std::time::Duration> = Vec::new();
    let mut t = Instant::now();
    for (i, (k, c)) in data.iter().enumerate() {
        rt.block_on(spectral_ingest::ingest::ingest_with(
            &format!("{i:016x}"),
            k,
            c,
            "note",
            0.0,
            "private",
            &spectral_ingest::ingest::IngestConfig::default(),
            &store_c,
            spectral_ingest::ingest::IngestOpts::default(),
        ))
        .unwrap();
        if (i + 1) % BUCKET == 0 {
            ing_buckets.push(t.elapsed());
            t = Instant::now();
        }
    }
    let one_tx_floor: std::time::Duration = ing_buckets.iter().sum();

    // ── D. Recognition enroll only, bucketed ───────────────────────────
    let tmp_d = tempfile::TempDir::new().unwrap();
    let store_d =
        spectral_recognition::SqliteRecognitionStore::open(&tmp_d.path().join("recognition.db"))
            .unwrap();
    let mut engine_d = spectral_recognition::RecognitionEngine::new(
        store_d,
        spectral_recognition::RecognitionConfig::default(),
    );
    let mut enr_buckets: Vec<std::time::Duration> = Vec::new();
    let mut t = Instant::now();
    for (i, (_k, c)) in data.iter().enumerate() {
        engine_d.enroll(&format!("{i:016x}"), c).unwrap();
        if (i + 1) % BUCKET == 0 {
            enr_buckets.push(t.elapsed());
            t = Instant::now();
        }
    }

    // ── E. Brain::remember WITH the fingerprint cap enabled ────────────
    let tmp_e = tempfile::TempDir::new().unwrap();
    let mut brain_e = open_brain_at(tmp_e.path());
    brain_e.set_max_fingerprint_peers(Some(spectral_ingest::ingest::DEFAULT_MAX_FINGERPRINT_PEERS));
    let mut capped_buckets: Vec<std::time::Duration> = Vec::new();
    let mut t = Instant::now();
    for (i, (k, c)) in data.iter().enumerate() {
        brain_e.remember(k, c, Visibility::Private).unwrap();
        if (i + 1) % BUCKET == 0 {
            capped_buckets.push(t.elapsed());
            t = Instant::now();
        }
    }
    let capped_total: std::time::Duration = capped_buckets.iter().sum();

    let per = |d: std::time::Duration| d.as_secs_f64() * 1000.0 / N as f64;
    println!("=== Write-path cost breakdown (N={N}, release) ===\n");
    println!(
        "A. remember, UNCAPPED (legacy)     {:>8.3} ms/mem   {:>8.1} ms total",
        per(public_path),
        public_path.as_secs_f64() * 1000.0
    );
    println!(
        "B. MemoryStore::write only         {:>8.3} ms/mem   {:>8.1} ms total",
        per(store_write_only),
        store_write_only.as_secs_f64() * 1000.0
    );
    println!(
        "C. ingest_with only                {:>8.3} ms/mem   {:>8.1} ms total",
        per(one_tx_floor),
        one_tx_floor.as_secs_f64() * 1000.0
    );
    println!();
    println!(
        "Derived-write overhead (A-B):      {:>8.3} ms/mem  ({:.0}% of public path)",
        per(public_path) - per(store_write_only),
        100.0 * (per(public_path) - per(store_write_only)) / per(public_path)
    );
    println!(
        "Per-transaction overhead (B-C):    {:>8.3} ms/mem",
        per(store_write_only) - per(one_tx_floor)
    );
    println!("\n--- per-memory cost by corpus position (does it grow?) ---");
    let pb = |d: std::time::Duration| d.as_secs_f64() * 1000.0 / BUCKET as f64;
    println!(
        "  {:>12}  {:>14}  {:>14}  {:>10}  {:>10}",
        "range", "remember", "remember+cap", "ingest", "enroll"
    );
    for (i, d) in buckets.iter().enumerate() {
        println!(
            "  {:>4}..{:<6}  {:>14.3}  {:>14.3}  {:>10.3}  {:>10.3}",
            i * BUCKET,
            (i + 1) * BUCKET,
            pb(*d),
            pb(capped_buckets[i]),
            pb(ing_buckets[i]),
            pb(enr_buckets[i])
        );
    }
    println!(
        "\n  remember TOTAL   uncapped {:.1} ms   capped {:.1} ms   ({:.0}% faster)",
        public_path.as_secs_f64() * 1000.0,
        capped_total.as_secs_f64() * 1000.0,
        100.0 * (public_path.as_secs_f64() - capped_total.as_secs_f64())
            / public_path.as_secs_f64()
    );
    println!(
        "  remember growth  uncapped {:.1}x   capped {:.1}x",
        pb(buckets[buckets.len() - 1]) / pb(buckets[0]),
        pb(capped_buckets[capped_buckets.len() - 1]) / pb(capped_buckets[0])
    );
    println!(
        "  enroll-only growth first->last: {:.1}x",
        pb(enr_buckets[enr_buckets.len() - 1]) / pb(enr_buckets[0])
    );
    println!(
        "  ingest-only growth first->last: {:.1}x",
        pb(ing_buckets[ing_buckets.len() - 1]) / pb(ing_buckets[0])
    );
    println!(
        "\nGrowth factor first->last bucket: {:.1}x",
        pb(buckets[buckets.len() - 1]) / pb(buckets[0])
    );
}
