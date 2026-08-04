//! Would TACT's constellation tier work if the wing classifier were real?
//!
//! Tier 1 fires on 3.2% of LongMemEval questions. The reason is not that the
//! constellation idea is weak — it is that `default_wing_rule_pairs()` ships
//! demo fixtures (`alice|coffee|noah|carol-doe`, `acme|widget|bob|recipe`), so
//! the taxonomy gating the tier was never built. This measures the headroom a
//! real taxonomy would unlock, before any is implemented.
//!
//! The taxonomy here is Spectral's own thesis applied to the wing problem:
//! **salient terms as topic anchors** — the same landmark/IDF idea the
//! recognition engine uses ("statistically salient features... the text analog
//! of spectral peaks above the noise floor"). A wing anchor is a stem that is
//! frequent enough to recur across sessions but rare enough to discriminate.
//!
//! Deterministic, $0, no model, no brains built.
//!
//! `cargo run -p spectral-bench-real --release --bin wing_taxonomy_probe -- <dataset.json>`

use std::collections::{HashMap, HashSet};

use spectral_ingest::{default_hall_rule_strings, default_wing_rule_strings};
use spectral_tact::classifier::{detect_hall, detect_wing};

/// Stems appearing in this fraction of documents or more are too common to
/// anchor a topic ("the", "time", "day").
const DF_CEILING: f64 = 0.010;
/// Stems appearing in fewer than this many documents cannot anchor a *recurring*
/// topic — they are one-offs, not areas.
const DF_FLOOR: usize = 40;
/// How many anchors the derived taxonomy keeps.
const MAX_WINGS: usize = 96;

/// Function words and conversational filler. A topic anchor has to name a
/// subject, not connect a sentence. Without this the "salient" band fills with
/// `just`, `want`, `could`, `most` — high document frequency, zero topicality.
const STOP: &[&str] = &[
    "just",
    "before",
    "after",
    "ensure",
    "want",
    "wanted",
    "making",
    "make",
    "many",
    "much",
    "area",
    "could",
    "would",
    "should",
    "recommendation",
    "recommend",
    "most",
    "during",
    "each",
    "check",
    "best",
    "example",
    "question",
    "know",
    "focu",
    "information",
    "popular",
    "look",
    "feel",
    "high",
    "happy",
    "really",
    "think",
    "thing",
    "things",
    "good",
    "great",
    "well",
    "also",
    "some",
    "more",
    "very",
    "like",
    "time",
    "times",
    "back",
    "even",
    "still",
    "need",
    "help",
    "sure",
    "here",
    "there",
    "when",
    "where",
    "what",
    "which",
    "your",
    "their",
    "them",
    "with",
    "from",
    "that",
    "this",
    "these",
    "those",
    "have",
    "has",
    "been",
    "were",
    "will",
    "about",
    "into",
    "over",
    "under",
    "then",
    "than",
    "other",
    "another",
    "such",
    "same",
    "different",
    "important",
    "consider",
    "including",
    "provide",
    "using",
    "used",
    "based",
    "start",
    "started",
    "keep",
    "take",
    "taken",
    "give",
    "given",
    "come",
    "came",
    "going",
    "getting",
    "trying",
    "little",
    "better",
    "always",
    "never",
    "often",
    "sometimes",
    "maybe",
    "actually",
    "definitely",
    "certainly",
    "perhap",
    "however",
    "although",
    "because",
];

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 3)
        .map(|w| w.trim_end_matches('s').to_string())
        .filter(|w| w.len() > 3 && !STOP.contains(&w.as_str()))
        .collect()
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: wing_taxonomy_probe <dataset.json>");
    let raw = std::fs::read_to_string(&path).expect("read dataset");
    let data: serde_json::Value = serde_json::from_str(&raw).expect("parse");
    let questions = data.as_array().expect("array");

    // ── Pass 1: document frequency over haystack turns ──
    let mut df: HashMap<String, usize> = HashMap::new();
    let mut docs = 0usize;
    for q in questions {
        let Some(sessions) = q.get("haystack_sessions").and_then(|v| v.as_array()) else {
            continue;
        };
        for session in sessions {
            let Some(turns) = session.as_array() else {
                continue;
            };
            for turn in turns {
                let Some(c) = turn.get("content").and_then(|c| c.as_str()) else {
                    continue;
                };
                docs += 1;
                for t in tokens(c).into_iter().collect::<HashSet<_>>() {
                    *df.entry(t).or_insert(0) += 1;
                }
            }
        }
    }

    // ── Derive anchors: mid-frequency stems, most discriminative first ──
    let ceiling = (docs as f64 * DF_CEILING) as usize;
    let mut candidates: Vec<(&String, &usize)> = df
        .iter()
        .filter(|(_, &n)| n >= DF_FLOOR && n <= ceiling)
        .collect();
    // Deterministic: by df desc, then stem asc.
    candidates.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let anchors: Vec<String> = candidates
        .into_iter()
        .take(MAX_WINGS)
        .map(|(s, _)| s.clone())
        .collect();
    let anchor_set: HashSet<&String> = anchors.iter().collect();

    // ── Measure reachability on QUESTIONS ──
    let wing_rules = default_wing_rule_strings();
    let hall_rules = default_hall_rule_strings();
    let (mut n, mut shipped_wing, mut shipped_both) = (0usize, 0usize, 0usize);
    let (mut derived_wing, mut derived_both) = (0usize, 0usize);

    for q in questions {
        let Some(text) = q.get("question").and_then(|v| v.as_str()) else {
            continue;
        };
        n += 1;
        let hall = detect_hall(text, &hall_rules);
        if detect_wing(text, &wing_rules).is_some() {
            shipped_wing += 1;
            if hall.is_some() {
                shipped_both += 1;
            }
        }
        let has_derived = tokens(text).iter().any(|t| anchor_set.contains(t));
        if has_derived {
            derived_wing += 1;
            if hall.is_some() {
                derived_both += 1;
            }
        }
    }

    let pct = |x: usize| 100.0 * x as f64 / n.max(1) as f64;
    println!("dataset: {path}");
    println!(
        "haystack turns scanned: {docs}   distinct stems: {}",
        df.len()
    );
    println!(
        "derived anchors: {} (df in [{DF_FLOOR}, {ceiling}])\n",
        anchors.len()
    );
    println!("{:<38} {:>10} {:>10}", "", "wing", "wing+hall");
    println!(
        "{:<38} {:>9.1}% {:>9.1}%   <- ships today (demo fixtures)",
        "shipped wing rules",
        pct(shipped_wing),
        pct(shipped_both)
    );
    println!(
        "{:<38} {:>9.1}% {:>9.1}%   <- corpus-derived anchors",
        "salient-term taxonomy",
        pct(derived_wing),
        pct(derived_both)
    );
    println!(
        "\ntier-1 reachability: {:.1}% -> {:.1}%  ({:.1}x)",
        pct(shipped_both),
        pct(derived_both),
        pct(derived_both) / pct(shipped_both).max(0.01)
    );
    println!("\nfirst 24 derived anchors:");
    for chunk in anchors.iter().take(24).collect::<Vec<_>>().chunks(6) {
        println!(
            "  {}",
            chunk
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}
