//! Phase A of the tier-1 ungating experiment — behavioural, on the real brain.
//!
//! Prereg: `docs/internal/tier1-ungating-prereg-2026-08-03.md`.
//!
//! Measures, over real Permagent queries against the real wing taxonomy:
//!   1. reachability — how often tier 1 fires, gated vs ungated
//!   2. non-degradation — does the ungated path return fewer results
//!   3. latency — median recall cost
//!   4. determinism — repeated queries byte-identical
//!
//! **This cannot show the change is good** — the brain has no ground-truth
//! answer keys. It shows the mechanism works and is safe. See the prereg.
//!
//! `cargo run -p spectral-bench-real --release --bin tier1_ungating -- \
//!     --brain <dir> --queries <file>`

use std::time::Instant;

use spectral_ingest::{default_hall_rule_strings, MemoryStore};
use spectral_tact::classifier::{detect_hall, detect_wing};
use spectral_tact::{extractor, RetrievalMethod, TactConfig};

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut dir, mut qfile) = (String::new(), String::new());
    while let Some(a) = args.next() {
        match a.as_str() {
            "--brain" => dir = args.next().unwrap_or_default(),
            "--queries" => qfile = args.next().unwrap_or_default(),
            _ => {}
        }
    }
    anyhow::ensure!(
        !dir.is_empty() && !qfile.is_empty(),
        "need --brain and --queries"
    );

    let store = spectral_ingest::sqlite_store::SqliteStore::open(
        &std::path::PathBuf::from(&dir).join("memory.db"),
    )?;

    // The real taxonomy, read from the brain itself — wings the consumer
    // actually uses, not a library default.
    let wings: Vec<String> = {
        let c = rusqlite::Connection::open(std::path::PathBuf::from(&dir).join("memory.db"))?;
        let mut st = c.prepare(
            "SELECT wing FROM memories WHERE wing IS NOT NULL AND wing != 'general' \
             GROUP BY wing HAVING COUNT(*) >= 10 ORDER BY wing",
        )?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.filter_map(Result::ok).collect()
    };
    let wing_rules: Vec<(String, String)> = wings
        .iter()
        .filter(|w| *w != "general")
        .map(|w| (regex::escape(w).replace("\\-", "[- ]?"), w.clone()))
        .collect();
    let hall_rules = default_hall_rule_strings();

    let queries: Vec<String> = std::fs::read_to_string(&qfile)?
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let gated = TactConfig {
        wing_rules: wing_rules.clone(),
        hall_rules: hall_rules.clone(),
        tier1_requires_hall: true,
        ..TactConfig::default()
    };
    let ungated = TactConfig {
        tier1_requires_hall: false,
        ..gated.clone()
    };

    let mut fired_g = 0usize;
    let mut fired_u = 0usize;
    let mut shrank = 0usize;
    let mut compared = 0usize;
    let (mut lat_g, mut lat_u) = (Vec::new(), Vec::new());
    let mut nondet = 0usize;
    let mut wing_detected = 0usize;
    let mut fp_empty = 0usize;

    for q in &queries {
        let w = detect_wing(q, &wing_rules);
        let h = detect_hall(q, &hall_rules);

        if w.is_some() {
            wing_detected += 1;
            // Did the constellation index have anything for this wing at all?
            let ww = w.clone().unwrap();
            let mut hs: Vec<String> = ["fact", "preference", "discovery", "advice", "event"]
                .iter()
                .flat_map(|a| extractor::query_hashes_for(a, &ww))
                .collect();
            hs.sort();
            hs.dedup();
            if store
                .fingerprint_search(&ww, "fact", &hs, 5)
                .await?
                .is_empty()
            {
                fp_empty += 1;
            }
        }

        let t = Instant::now();
        let (rg, mg) = extractor::search(q, &w, &h, &gated, &store).await?;
        lat_g.push(t.elapsed().as_secs_f64() * 1000.0);

        let t = Instant::now();
        let (ru, mu) = extractor::search(q, &w, &h, &ungated, &store).await?;
        lat_u.push(t.elapsed().as_secs_f64() * 1000.0);

        if mg == RetrievalMethod::Fingerprint {
            fired_g += 1;
        }
        if mu == RetrievalMethod::Fingerprint {
            fired_u += 1;
            compared += 1;
            if ru.len() < rg.len() {
                shrank += 1;
            }
        }

        // Determinism on the ungated path.
        let (ru2, _) = extractor::search(q, &w, &h, &ungated, &store).await?;
        let k = |v: &Vec<spectral_ingest::MemoryHit>| {
            v.iter().map(|h| h.id.clone()).collect::<Vec<_>>()
        };
        if k(&ru) != k(&ru2) {
            nondet += 1;
        }
    }

    let n = queries.len().max(1);
    let pct = |x: usize| 100.0 * x as f64 / n as f64;
    let (mg, mu) = (median(lat_g), median(lat_u));

    println!("brain:   {dir}");
    println!(
        "wing detected on {} of {n} queries ({:.1}%); of those, constellation index EMPTY for {fp_empty}",
        wing_detected,
        100.0 * wing_detected as f64 / n as f64
    );
    println!(
        "queries: {n}   real wings in taxonomy: {}\n",
        wing_rules.len()
    );
    println!(
        "{:<38} {:>10}",
        "tier 1 fires — GATED (wing AND hall)",
        format!("{:.1}%", pct(fired_g))
    );
    println!(
        "{:<38} {:>10}",
        "tier 1 fires — UNGATED (wing only)",
        format!("{:.1}%", pct(fired_u))
    );
    println!(
        "\ngate 1 reachability >= 30%:      {}",
        if pct(fired_u) >= 30.0 { "PASS" } else { "FAIL" }
    );
    let ok_shrink = compared == 0 || (compared - shrank) as f64 / compared as f64 >= 0.95;
    println!(
        "gate 2 non-degradation >= 95%:   {}  ({} of {} kept size)",
        if ok_shrink { "PASS" } else { "FAIL" },
        compared - shrank,
        compared
    );
    let lat_delta = (mu - mg) / mg * 100.0;
    println!(
        "gate 3 latency <= +20%:          {}  ({mg:.3} -> {mu:.3} ms, {lat_delta:+.1}%)",
        if lat_delta <= 20.0 { "PASS" } else { "FAIL" }
    );
    println!(
        "gate 4 determinism:              {}  ({nondet} non-deterministic)",
        if nondet == 0 { "PASS" } else { "FAIL" }
    );
    Ok(())
}
