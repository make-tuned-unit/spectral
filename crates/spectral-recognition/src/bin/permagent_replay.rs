//! Real Permagent outcome replay — recognition against production behaviour.
//!
//! The synthetic replays (degrade/paraphrase) prove discrimination on
//! constructed positives. This one asks a different, real-world question on
//! actual production queries: when the deterministic recognition engine is fed
//! a real user query, does it recognise the *right* memories?
//!
//! Two labels come from production, no synthesis:
//!  - **wing precision** — the query's real project focus (`rc_focus_wing`).
//!    When the engine's top trace is a memory, is that memory in the same wing
//!    the agent was actually working in?
//!  - **cascade agreement** — the memories the production recall cascade
//!    retrieved for this query (`recognition_set_members`). Does the engine's
//!    top trace fall inside that set? High agreement means the model-free
//!    engine surfaces what the production system already found relevant.
//!
//! Honest limitation (measured, this snapshot): `recognition_events` has 149
//! outcomes, ALL Positive — no negative outcomes and no per-query familiar/novel
//! label. So a discrimination AUC is NOT computable here; that needs Permagent
//! to emit negative outcomes. This replay measures relevance agreement, which
//! the data does support.
//!
//! Usage: permagent_replay --brain <memory.db> --events <permagent.db> [--limit N]

use anyhow::{Context, Result};
use spectral_recognition::{
    InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine, Verdict,
};
use std::collections::{HashMap, HashSet};

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let brain_path = arg(&args, "--brain").context("--brain <memory.db> required")?;
    let events_path = arg(&args, "--events").context("--events <permagent.db> required")?;
    let limit: usize = arg(&args, "--limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);

    // Brain: enroll every memory, and remember each memory's wing.
    let brain = rusqlite::Connection::open_with_flags(
        brain_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut stmt = brain.prepare(
        "SELECT id, content, COALESCE(wing, '') FROM memories WHERE LENGTH(content) >= 60",
    )?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let wing_of: HashMap<String, String> = rows
        .iter()
        .map(|(id, _, w)| (id.clone(), w.clone()))
        .collect();

    let mut engine = RecognitionEngine::new(
        InMemoryRecognitionStore::default(),
        RecognitionConfig::default(),
    );
    for (id, content, _) in &rows {
        engine.enroll(id, content)?;
    }
    eprintln!("enrolled {} brain memories", rows.len());

    // Events DB: the production cascade's retrieved set per query.
    let ev = rusqlite::Connection::open_with_flags(
        events_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut sm = ev.prepare("SELECT retrieval_id, memory_id FROM recognition_set_members")?;
    let mut retrieved: HashMap<String, HashSet<String>> = HashMap::new();
    for row in sm.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (rid, mid) = row?;
        retrieved.entry(rid).or_default().insert(mid);
    }

    // Substantive queries with a real project focus.
    let mut eq = ev.prepare(
        "SELECT retrieval_id, query, COALESCE(rc_focus_wing, '') \
         FROM recognition_events \
         WHERE rc_focus_wing IS NOT NULL AND rc_focus_wing != '' AND LENGTH(query) >= 30 \
         ORDER BY retrieval_id",
    )?;
    let events: Vec<(String, String, String)> = eq
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let events: Vec<_> = events.into_iter().take(limit).collect();

    let (mut non_novel, mut wing_hits, mut wing_evaluable) = (0usize, 0usize, 0usize);
    let (mut cascade_hits, mut cascade_evaluable) = (0usize, 0usize);
    for (rid, query, focus_wing) in &events {
        let r = engine.recognize(query)?;
        if !matches!(r.verdict, Verdict::Novel) {
            non_novel += 1;
        }
        let Some(top) = r.traces.first() else {
            continue;
        };

        // Wing precision: is the top recognised memory in the query's wing?
        if let Some(w) = wing_of.get(&top.memory_id) {
            if !w.is_empty() {
                wing_evaluable += 1;
                if w == focus_wing {
                    wing_hits += 1;
                }
            }
        }
        // Cascade agreement: is the top trace in the production retrieved set?
        if let Some(set) = retrieved.get(rid) {
            cascade_evaluable += 1;
            if set.contains(&top.memory_id) {
                cascade_hits += 1;
            }
        }
    }

    let n = events.len();
    println!("== permagent real-query recognition replay ==");
    println!("events (substantive + wing): {n}");
    println!(
        "recognised as non-novel:     {non_novel} ({:.1}%)",
        100.0 * non_novel as f64 / n.max(1) as f64
    );
    println!(
        "wing precision (top trace):  {wing_hits}/{wing_evaluable} ({:.1}%)",
        100.0 * wing_hits as f64 / wing_evaluable.max(1) as f64
    );
    println!(
        "cascade agreement (top∈set): {cascade_hits}/{cascade_evaluable} ({:.1}%)",
        100.0 * cascade_hits as f64 / cascade_evaluable.max(1) as f64
    );
    println!(
        "\nNOTE: all {n} carry Positive outcomes only — no negative outcomes in\n\
         this snapshot, so discrimination AUC is not computable here. These are\n\
         relevance-agreement metrics on real production queries."
    );
    Ok(())
}
