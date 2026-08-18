//! Which of Spectral's capabilities is a given brain actually feeding?
//!
//! Spectral offers six kinds of memory, but each one needs particular data to
//! be present and *varied* before it can do anything. A field that is null, or
//! constant, silently disables the engine that reads it — no error, no warning,
//! just a capability that quietly does nothing. This audit reports, per
//! capability, whether the substrate is there.
//!
//! Run:
//! ```text
//! cargo run -p spectral --example brain_audit -- ~/.permagent/brain
//! ```
//!
//! **Privacy: statistics only.** Field *values* are printed for low-cardinality
//! routing keys (`wing`, `hall`, `visibility`) because their distribution is
//! the finding; no memory content, description, key or id is ever printed.
//! Every database is opened read-only.

use std::collections::HashMap;
use std::path::Path;

/// Shannon entropy in bits, and the same normalised against the maximum for
/// this many categories. Normalised entropy is the useful number: 1.0 means
/// perfectly even, 0.0 means one value dominates completely.
fn entropy(counts: &HashMap<String, i64>) -> (f64, f64, f64) {
    let total: i64 = counts.values().sum();
    if total == 0 || counts.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let h: f64 = counts
        .values()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / total as f64;
            -p * p.log2()
        })
        .sum();
    let max_h = (counts.len() as f64).log2();
    let norm = if max_h > 0.0 { h / max_h } else { 0.0 };
    let top = *counts.values().max().unwrap_or(&0) as f64 / total as f64;
    (h, norm, top)
}

fn open_ro(path: &Path) -> rusqlite::Result<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
}

fn distribution(conn: &rusqlite::Connection, sql: &str) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(sql) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?
                    .unwrap_or_else(|| "(null)".into()),
                r.get::<_, i64>(1)?,
            ))
        }) {
            for row in rows.flatten() {
                out.insert(row.0, row.1);
            }
        }
    }
    out
}

fn pct(n: i64, of: i64) -> String {
    if of <= 0 {
        return "n/a".into();
    }
    format!("{:.1}%", 100.0 * n as f64 / of as f64)
}

/// A one-line verdict, so the report says what to *do*, not just what is.
fn verdict(ok: bool, live: &str, dead: &str) -> String {
    if ok {
        format!("LIVE     {live}")
    } else {
        format!("DEGRADED {dead}")
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .ok_or("usage: brain_audit <brain-dir>")?;
    let dir = Path::new(&dir);

    let mem = open_ro(&dir.join("memory.db"))?;
    let graph = open_ro(&dir.join("graph.sqlite")).ok();
    let recog = open_ro(&dir.join("recognition.db")).ok();

    let total = count(&mem, "SELECT COUNT(*) FROM memories");
    println!("Brain audit — {}", dir.display());
    println!("  memories: {total}\n");

    // ── Recall: TACT routing ────────────────────────────────────────
    // The TACT fingerprint is hash(hall, target_hall, wing, time_bucket). A
    // routing key that is nearly constant contributes almost no entropy, so
    // the fingerprint channel degenerates and recall leans entirely on FTS.
    println!("── RECALL (TACT fingerprint routing) ──");
    for (field, sql) in [
        ("wing", "SELECT wing, COUNT(*) FROM memories GROUP BY wing"),
        ("hall", "SELECT hall, COUNT(*) FROM memories GROUP BY hall"),
    ] {
        let d = distribution(&mem, sql);
        let (h, norm, top) = entropy(&d);
        let top_name = d
            .iter()
            .max_by_key(|(_, &v)| v)
            .map(|(k, _)| k.clone())
            .unwrap_or_default();
        println!(
            "  {field:<6} {:>3} values  H={h:.2} bits  normalised={norm:.2}  top={top_name:?} at {}",
            d.len(),
            format_args!("{:.1}%", 100.0 * top)
        );
        // `hall` deserves a sharper reading than "one value dominates". No
        // default rule produces "event" — it is purely `classify_hall`'s
        // fallback — so an "event" share IS the share of the corpus that
        // matched no rule at all. Reporting that as a dominant category would
        // hide a coverage failure behind what looks like a classification.
        let fallthrough = if field == "hall" {
            d.get("event").copied().unwrap_or(0)
        } else {
            d.get("general").copied().unwrap_or(0)
        };
        let total_rows: i64 = d.values().sum();
        if total_rows > 0 {
            let fb = if field == "hall" { "event" } else { "general" };
            println!(
                "         {} is the CATCH-ALL: {} of memories matched no {field} rule",
                fb,
                pct(fallthrough, total_rows)
            );
        }
        println!(
            "         {}",
            verdict(
                norm >= 0.5 && top < 0.6,
                "well spread — the fingerprint discriminates",
                "the catch-all dominates — this key contributes little routing \
                 signal, and the taxonomy does not cover this corpus"
            )
        );
    }

    // ── Recognition ─────────────────────────────────────────────────
    println!("\n── RECOGNITION (\"have I seen this before?\") ──");
    if let Some(r) = &recog {
        let enrolled = count(r, "SELECT COUNT(*) FROM recognition_enrolled");
        println!(
            "  enrolled: {enrolled} of {total} ({})",
            pct(enrolled, total)
        );
        println!(
            "  pairs: {}",
            count(r, "SELECT COUNT(*) FROM recognition_pairs")
        );
        println!(
            "  grams: {}",
            count(r, "SELECT COUNT(*) FROM recognition_grams")
        );
        println!(
            "  {}",
            verdict(
                total > 0 && enrolled as f64 / total as f64 >= 0.95,
                "the whole corpus can answer a recognition query",
                "unenrolled memories are INVISIBLE to recognition — it cannot \
                 recognise what was never indexed"
            )
        );
    } else {
        println!("  no recognition.db — recognition is entirely unavailable");
    }

    // ── Relational graph ────────────────────────────────────────────
    println!("\n── RELATIONAL (typed graph, 2-hop traversal) ──");
    if let Some(g) = &graph {
        let entities = count(g, "SELECT COUNT(*) FROM entity");
        let triples = count(g, "SELECT COUNT(*) FROM triple");
        let docs = count(g, "SELECT COUNT(*) FROM document");
        let mentions = count(g, "SELECT COUNT(*) FROM mention");
        println!("  entities: {entities}   triples: {triples}");
        println!("  documents: {docs}   mentions: {mentions}");
        let per_entity = if entities > 0 {
            triples as f64 / entities as f64
        } else {
            0.0
        };
        println!("  edges per entity: {per_entity:.2}");
        println!(
            "  {}",
            verdict(
                per_entity >= 1.0,
                "there is a graph to traverse",
                "almost no edges — 2-hop traversal, spreading activation and \
                 related_memories have nothing to walk. Triples arrive only via \
                 assert() or LLM extraction; neither is running at scale"
            )
        );
    } else {
        println!("  no graph.sqlite");
    }

    // ── Episodic ────────────────────────────────────────────────────
    println!("\n── EPISODIC / TEMPORAL ──");
    let with_ep = count(
        &mem,
        "SELECT COUNT(*) FROM memories WHERE episode_id IS NOT NULL AND TRIM(episode_id)<>''",
    );
    let episodes = count(
        &mem,
        "SELECT COUNT(DISTINCT episode_id) FROM memories WHERE episode_id IS NOT NULL",
    );
    let days = count(
        &mem,
        "SELECT COUNT(DISTINCT substr(created_at,1,10)) FROM memories",
    );
    println!(
        "  episode_id set: {with_ep} ({})   distinct episodes: {episodes}",
        pct(with_ep, total)
    );
    println!("  distinct days: {days}");
    println!(
        "  {}",
        verdict(
            days > 7 && with_ep > 0,
            "time buckets and episodes both discriminate",
            "too little temporal spread for recency or episode grouping to matter"
        )
    );

    // ── Adaptive ────────────────────────────────────────────────────
    println!("\n── ADAPTIVE (use strengthens ranking) ──");
    let distinct_scores = count(&mem, "SELECT COUNT(DISTINCT signal_score) FROM memories");
    let at_default = count(
        &mem,
        "SELECT COUNT(*) FROM memories WHERE signal_score = 1.0",
    );
    println!(
        "  distinct signal_score values: {distinct_scores}   still at default 1.0: {at_default} ({})",
        pct(at_default, total)
    );
    println!(
        "  {}",
        verdict(
            distinct_scores > 10,
            "reinforcement is happening and the score discriminates",
            "signal_score is near-constant — adaptive re-ranking is inert"
        )
    );

    // ── Integrity / provenance ──────────────────────────────────────
    println!("\n── INTEGRITY & PROVENANCE ──");
    let hashed = count(
        &mem,
        "SELECT COUNT(*) FROM memories WHERE content_hash IS NOT NULL",
    );
    let signed = count(
        &mem,
        "SELECT COUNT(*) FROM memories WHERE signature IS NOT NULL",
    );
    println!("  content_hash: {hashed} ({})", pct(hashed, total));
    println!("  signature:    {signed} ({})", pct(signed, total));
    println!(
        "  {}",
        verdict(
            total > 0 && signed as f64 / total as f64 >= 0.95,
            "provenance is complete",
            "unsigned memories cannot be authenticated when shared, and \
             `repair_derivations()` exists to backfill both fields"
        )
    );

    // ── Visibility / federation ─────────────────────────────────────
    println!("\n── VISIBILITY & FEDERATION ──");
    let vis = distribution(
        &mem,
        "SELECT visibility, COUNT(*) FROM memories GROUP BY visibility",
    );
    let mut labels: Vec<_> = vis.iter().collect();
    labels.sort_by_key(|(_, &v)| -v);
    for (k, v) in &labels {
        println!("  {k:<10} {v:>6} ({})", pct(**v, total));
    }
    println!(
        "  {}",
        verdict(
            labels.len() > 1,
            "scoping distinguishes what may be shared",
            "every memory has the same visibility — scoping and federation are \
             inert here (expected for a purely personal brain)"
        )
    );
    Ok(())
}
