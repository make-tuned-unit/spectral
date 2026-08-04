//! How often does conservative supersession extraction actually fire?
//!
//! Gate 3 of `docs/internal/supersession-prereg-2026-08-03.md` asks whether the
//! lever fires at all. Context-hash diffs cannot answer that when the lever
//! also widens the candidate pool, so this measures extraction directly over
//! the dataset: no retrieval, no brains, no confound.
//!
//! `cargo run -p spectral --release --example supersession_coverage -- <dataset.json>`

use spectral::supersession::topic_key;
use std::collections::{HashMap, HashSet};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: supersession_coverage <dataset.json>");
    let raw = std::fs::read_to_string(&path).expect("read dataset");
    let data: serde_json::Value = serde_json::from_str(&raw).expect("parse dataset");
    let questions = data.as_array().expect("dataset is an array");

    let mut turns_total = 0usize;
    let mut turns_with_topic = 0usize;
    // question -> topic -> set of sessions asserting it
    let mut q_with_conflict = 0usize;
    let mut conflicts_total = 0usize;
    let mut examples: Vec<String> = Vec::new();

    for q in questions {
        let sessions = q.get("haystack_sessions").and_then(|v| v.as_array());
        let Some(sessions) = sessions else { continue };
        let mut topic_sessions: HashMap<String, HashSet<usize>> = HashMap::new();

        for (si, session) in sessions.iter().enumerate() {
            let Some(turns) = session.as_array() else {
                continue;
            };
            for turn in turns {
                let Some(content) = turn.get("content").and_then(|c| c.as_str()) else {
                    continue;
                };
                turns_total += 1;
                if let Some(topic) = topic_key(content) {
                    turns_with_topic += 1;
                    topic_sessions.entry(topic.clone()).or_default().insert(si);
                    if examples.len() < 8 {
                        let snippet: String = content.chars().take(90).collect();
                        examples.push(format!("[{topic}] {snippet}"));
                    }
                }
            }
        }

        let conflicts = topic_sessions.values().filter(|s| s.len() > 1).count();
        if conflicts > 0 {
            q_with_conflict += 1;
            conflicts_total += conflicts;
        }
    }

    println!("dataset: {path}");
    println!("questions:            {}", questions.len());
    println!("haystack turns:       {turns_total}");
    println!(
        "turns with a topic:   {turns_with_topic}  ({:.2}%)",
        100.0 * turns_with_topic as f64 / turns_total.max(1) as f64
    );
    println!(
        "questions where one topic is asserted in >1 session (suppressible): {q_with_conflict}  ({:.1}%)",
        100.0 * q_with_conflict as f64 / questions.len().max(1) as f64
    );
    println!("total suppressible topic groups: {conflicts_total}");
    println!("\nsample extractions:");
    for e in &examples {
        println!("  {e}");
    }
}
