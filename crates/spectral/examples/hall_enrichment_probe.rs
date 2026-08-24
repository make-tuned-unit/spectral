//! R39 probe — does the Librarian description give TACT a hall it cannot get
//! from the content?
//!
//! `classify_hall` runs on `content` at write time; on the real brain 77.7% of
//! memories match no default rule and fall to `event`. TACT's tier‑1 fingerprint
//! search needs a real hall on the memory. This measures, read‑only, what the
//! default rules produce from three texts per described memory: content,
//! content+description, description alone. Prints the hall distribution and
//! the fallback rate for each. $0, seconds.
//!
//! usage: hall_enrichment_probe <memory.db>

use spectral_ingest::classifier::{classify_hall, default_hall_rules};
use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = std::env::args()
        .nth(1)
        .ok_or("usage: hall_enrichment_probe <memory.db>")?;
    let conn = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let mut stmt = conn.prepare(
        "SELECT content, description FROM memories
         WHERE content IS NOT NULL AND TRIM(content) <> ''
           AND description IS NOT NULL AND TRIM(description) <> ''",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let rules = default_hall_rules();
    let mut dist: [BTreeMap<String, usize>; 3] = Default::default();
    let mut changed = 0usize; // content says event, enriched says something else
    let mut conflict = 0usize; // both non-event and different
    for (content, desc) in &rows {
        let enriched = format!("{content}\n{desc}");
        let hs = [
            classify_hall(content, &rules),
            classify_hall(&enriched, &rules),
            classify_hall(desc, &rules),
        ];
        for (i, h) in hs.iter().enumerate() {
            *dist[i].entry(h.clone()).or_insert(0) += 1;
        }
        if hs[0] == "event" && hs[1] != "event" {
            changed += 1;
        }
        if hs[0] != "event" && hs[2] != "event" && hs[0] != hs[2] {
            conflict += 1;
        }
    }
    let n = rows.len().max(1) as f64;
    println!(
        "R39 hall enrichment probe — {} described memories, default hall rules",
        rows.len()
    );
    for (i, name) in ["content", "content+desc", "desc only"].iter().enumerate() {
        let fallback = *dist[i].get("event").unwrap_or(&0) as f64 / n;
        let mut ent = 0.0;
        for c in dist[i].values() {
            let p = *c as f64 / n;
            ent -= p * p.log2();
        }
        let k = dist[i].len().max(2) as f64;
        println!(
            "  {name:<13} fallback(event) {:5.1}%  halls {}  normalised entropy {:.2}  {:?}",
            100.0 * fallback,
            dist[i].len(),
            ent / k.log2(),
            dist[i]
        );
    }
    println!(
        "  content=event but content+desc != event : {} ({:.1}%)",
        changed,
        100.0 * changed as f64 / n
    );
    println!(
        "  content and desc both non-event but DIFFERENT: {} ({:.1}%)",
        conflict,
        100.0 * conflict as f64 / n
    );
    Ok(())
}
