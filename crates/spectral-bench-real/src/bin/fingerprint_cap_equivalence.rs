//! Does capping constellation fingerprint fan-out change what TACT retrieves?
//!
//! Deterministic, $0, no LLM. Builds two stores from an identical corpus —
//! one with unbounded pairing (legacy), one with the capped default — then
//! runs the TACT tier-1 path (`spectral_tact::extractor::search`, the only
//! consumer of these edges) over the same queries and diffs the ordered hit
//! keys. Any divergence is reported per query, not summarised away.

use spectral_ingest::ingest::{ingest_with, IngestConfig, IngestOpts};
use spectral_ingest::sqlite_store::SqliteStore;
use spectral_ingest::MemoryStore;

const N: usize = 600;

fn corpus(n: usize) -> Vec<(String, String)> {
    // Deliberately spread across halls/wings so fingerprint hashes vary,
    // and include repeated topical language so tier-1 actually fires.
    let topics = [
        ("deploy the release to the staging cluster", "work"),
        ("the sprint retrospective covered open bugs", "work"),
        ("roast garlic and salt for the tomato sauce", "cooking"),
        ("cycled twenty kilometres along the coast", "fitness"),
        ("read two chapters of the systems book", "reading"),
        ("call with the dentist about the appointment", "health"),
    ];
    (0..n)
        .map(|i| {
            let (text, topic) = topics[i % topics.len()];
            (
                format!("{topic}:note:{i}"),
                format!("Entry {i}: {text}, noted for team {}.", i % 4),
            )
        })
        .collect()
}

fn build(cap: Option<usize>, data: &[(String, String)], dir: &std::path::Path) -> SqliteStore {
    let store = SqliteStore::open(&dir.join("memory.db")).unwrap();
    let config = IngestConfig {
        max_fingerprint_peers: cap,
        ..IngestConfig::default()
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    for (i, (k, c)) in data.iter().enumerate() {
        rt.block_on(ingest_with(
            &format!("{i:016x}"),
            k,
            c,
            "note",
            0.0,
            "private",
            &config,
            &store,
            IngestOpts::default(),
        ))
        .unwrap();
    }
    store
}

fn edge_count(dir: &std::path::Path) -> i64 {
    // Separate read connection: the store's own handle is crate-private.
    rusqlite::Connection::open(dir.join("memory.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM constellation_fingerprints", [], |r| {
            r.get(0)
        })
        .unwrap()
}

fn main() {
    let data = corpus(N);
    let tmp_u = tempfile::TempDir::new().unwrap();
    let tmp_c = tempfile::TempDir::new().unwrap();

    let uncapped = build(None, &data, tmp_u.path());
    let capped = build(Some(64), &data, tmp_c.path());

    let edges_u = edge_count(tmp_u.path());
    let edges_c = edge_count(tmp_c.path());

    // Query set: the same phrasing a consumer would send, across every wing.
    let queries: Vec<(&str, &str, &str)> = vec![
        ("what did we deploy", "work", "task"),
        ("open bugs from the retrospective", "work", "task"),
        ("how do I make the sauce", "cooking", "task"),
        ("how far did I cycle", "fitness", "event"),
        ("what am I reading", "reading", "event"),
        ("dentist appointment", "health", "event"),
        ("team notes", "work", "fact"),
        ("coast ride", "fitness", "fact"),
    ];

    let rt = tokio::runtime::Runtime::new().unwrap();
    let cfg = spectral_tact::TactConfig::default();
    let mut diverged = 0usize;

    println!("=== Fingerprint cap equivalence (N={N}, TACT tier-1 path) ===\n");
    println!("constellation edges: uncapped {edges_u}  capped {edges_c}");
    if edges_u > 0 {
        println!(
            "edge reduction:      {:.1}% fewer rows\n",
            100.0 * (edges_u - edges_c) as f64 / edges_u as f64
        );
    }

    for (q, wing, hall) in &queries {
        let run = |s: &dyn MemoryStore| -> (Vec<String>, String) {
            let (hits, method) = rt
                .block_on(spectral_tact::extractor::search(
                    q,
                    &Some(wing.to_string()),
                    &Some(hall.to_string()),
                    &cfg,
                    s,
                ))
                .unwrap();
            (
                hits.iter().map(|h| h.key.clone()).collect(),
                format!("{method:?}"),
            )
        };
        let (ku, mu) = run(&uncapped);
        let (kc, mc) = run(&capped);
        let same = ku == kc && mu == mc;
        if !same {
            diverged += 1;
        }
        println!(
            "{} q={q:<38} method={mu:<12} hits={:<3} {}",
            if same { "SAME  " } else { "DIFFER" },
            ku.len(),
            if same {
                String::new()
            } else {
                format!("\n        uncapped={ku:?}\n        capped  ={kc:?} method={mc}")
            }
        );
    }

    println!(
        "\n{}/{} pipeline queries diverged.",
        diverged,
        queries.len()
    );

    // ── Direct consumer test ───────────────────────────────────────────
    // The pipeline above fell through to FTS, so tier-1 was never exercised.
    // Drive `fingerprint_search` directly with hashes taken from the stored
    // edges, so the comparison actually reaches the rows the cap removes.
    println!("\n=== Direct fingerprint_search comparison (real stored hashes) ===");
    let conn = rusqlite::Connection::open(tmp_u.path().join("memory.db")).unwrap();
    let probes: Vec<(String, String, String)> = conn
        .prepare(
            "SELECT DISTINCT fingerprint_hash, wing, anchor_hall
             FROM constellation_fingerprints ORDER BY fingerprint_hash LIMIT 12",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    let mut direct_div = 0usize;
    let mut fired = 0usize;
    for (hash, wing, hall) in &probes {
        let go = |s: &dyn MemoryStore| -> Vec<String> {
            rt.block_on(s.fingerprint_search(wing, hall, std::slice::from_ref(hash), 20))
                .unwrap()
                .iter()
                .map(|h| h.key.clone())
                .collect()
        };
        let a = go(&uncapped);
        let b = go(&capped);
        if !a.is_empty() || !b.is_empty() {
            fired += 1;
        }
        if a != b {
            direct_div += 1;
            println!(
                "  DIFFER hash={} wing={wing} hall={hall}",
                &hash[..8.min(hash.len())]
            );
            println!("     uncapped ({:>3}): {:?}", a.len(), &a[..a.len().min(6)]);
            println!("     capped   ({:>3}): {:?}", b.len(), &b[..b.len().min(6)]);
        }
    }
    println!(
        "  probes={} fired={} diverged={}",
        probes.len(),
        fired,
        direct_div
    );
    if fired == 0 {
        println!("  NOTE: no probe returned rows in either arm - test is vacuous.");
    }
}
