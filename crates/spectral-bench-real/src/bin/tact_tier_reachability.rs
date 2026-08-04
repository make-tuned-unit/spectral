//! Can TACT's fingerprint tier fire at all on the real benchmark corpora?
//!
//! Tier 1 (`constellation_fingerprints`, the table that costs ~39% of a write
//! and ~57% of store bytes) runs only when BOTH a wing and a hall are detected
//! on the query:
//!
//! ```ignore
//! if let (Some(w), Some(h)) = (wing, hall) { ... fingerprint_search ... }
//! ```
//!
//! Tier 2 needs a wing. So if wing detection is rare, both tiers are
//! unreachable and the fingerprint table has no production reader — making its
//! entire write and storage cost dead weight.
//!
//! Deterministic, $0, no LLM, no brains built. Reads the dataset and runs the
//! shipped classifier over every question.
//!
//! `cargo run -p spectral-bench-real --release --bin tact_tier_reachability -- <dataset.json>`

use spectral_ingest::{default_hall_rule_strings, default_wing_rule_strings};
use spectral_tact::classifier::{detect_hall, detect_wing};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: tact_tier_reachability <dataset.json>");
    let raw = std::fs::read_to_string(&path).expect("read dataset");
    let data: serde_json::Value = serde_json::from_str(&raw).expect("parse dataset");
    let questions = data.as_array().expect("dataset is an array");

    let wing_rules = default_wing_rule_strings();
    let hall_rules = default_hall_rule_strings();

    let (mut n, mut wing_hit, mut hall_hit, mut both) = (0usize, 0usize, 0usize, 0usize);
    let mut examples: Vec<String> = Vec::new();

    for q in questions {
        let Some(text) = q.get("question").and_then(|v| v.as_str()) else {
            continue;
        };
        n += 1;
        let w = detect_wing(text, &wing_rules);
        let h = detect_hall(text, &hall_rules);
        if w.is_some() {
            wing_hit += 1;
        }
        if h.is_some() {
            hall_hit += 1;
        }
        if let (Some(w), Some(h)) = (&w, &h) {
            both += 1;
            if examples.len() < 5 {
                let snippet: String = text.chars().take(80).collect();
                examples.push(format!("[{w}/{h}] {snippet}"));
            }
        }
    }

    let pct = |x: usize| 100.0 * x as f64 / n.max(1) as f64;
    println!("dataset: {path}");
    println!("questions:                     {n}");
    println!(
        "wing detected:                 {wing_hit}  ({:.1}%)",
        pct(wing_hit)
    );
    println!(
        "hall detected:                 {hall_hit}  ({:.1}%)",
        pct(hall_hit)
    );
    println!(
        "BOTH -> tier 1 reachable:      {both}  ({:.1}%)   <-- fingerprint table's only production reader",
        pct(both)
    );
    println!(
        "wing only -> tier 2 reachable: {}  ({:.1}%)",
        wing_hit.saturating_sub(both),
        pct(wing_hit.saturating_sub(both))
    );
    if !examples.is_empty() {
        println!("\nsample tier-1-reachable questions:");
        for e in &examples {
            println!("  {e}");
        }
    }
}
