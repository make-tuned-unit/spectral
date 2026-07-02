//! Real-data recognition replay: enroll a brain's memories, then measure
//! familiar-vs-novel discrimination with zero LLM calls.
//!
//! Protocol (honest by construction):
//! - Split memories deterministically 90/10 (hash of id).
//! - Enroll the 90% ("known").
//! - Positives: degraded copies of enrolled memories (deterministic ~30%
//!   token dropout — the Shazam noisy-fragment condition).
//! - Negatives: the held-out 10% — true novels drawn from the SAME
//!   distribution (much harder than off-topic negatives).
//! - Report AUC over familiarity, verdict confusion, and latency.
//!
//! Usage: replay --db <path/to/memory.db> [--limit N]

use anyhow::{Context, Result};
use spectral_recognition::{
    InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine, Verdict,
};

fn hash_id(id: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(id.as_bytes());
    u64::from_be_bytes(d[..8].try_into().unwrap())
}

/// Deterministic ~drop_pct token dropout keyed on (memory id, position).
fn degrade(content: &str, id: &str, drop_pct: u64) -> String {
    content
        .split_whitespace()
        .enumerate()
        .filter(|(i, _)| hash_id(&format!("{id}|{i}")) % 100 >= drop_pct)
        .map(|(_, t)| t)
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let db_path = args
        .iter()
        .position(|a| a == "--db")
        .and_then(|i| args.get(i + 1))
        .context("--db <path> required")?;
    let limit: usize = args
        .iter()
        .position(|a| a == "--limit")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);

    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut stmt =
        conn.prepare("SELECT id, content FROM memories WHERE LENGTH(content) >= 60 ORDER BY id")?;
    let memories: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let memories: Vec<_> = memories.into_iter().take(limit).collect();

    let (known, held_out): (Vec<_>, Vec<_>) =
        memories.iter().partition(|(id, _)| hash_id(id) % 10 != 0);
    eprintln!(
        "memories={} enrolled={} held_out={}",
        memories.len(),
        known.len(),
        held_out.len()
    );

    let mut engine = RecognitionEngine::new(
        InMemoryRecognitionStore::default(),
        RecognitionConfig::default(),
    );
    let t = std::time::Instant::now();
    for (id, content) in &known {
        engine.enroll(id, content)?;
    }
    eprintln!(
        "enroll: {:.1}ms total, {:.3}ms/memory",
        t.elapsed().as_secs_f64() * 1e3,
        t.elapsed().as_secs_f64() * 1e3 / known.len().max(1) as f64
    );

    // Positives: degraded enrolled memories. Track both scalar candidates.
    let mut scores: Vec<(f64, f64, bool)> = Vec::new(); // (familiarity, odds, is_old)
    let mut pos_recognized = 0usize;
    let mut pos_correct_trace = 0usize;
    let mut pos_novel = 0usize;
    let t = std::time::Instant::now();
    for (id, content) in &known {
        let stimulus = degrade(content, id, 30);
        if stimulus.split_whitespace().count() < 5 {
            continue;
        }
        let r = engine.recognize(&stimulus)?;
        scores.push((r.familiarity, r.odds_of_old, true));
        match &r.verdict {
            Verdict::Recognized { memory_id } => {
                pos_recognized += 1;
                if memory_id == id {
                    pos_correct_trace += 1;
                }
            }
            Verdict::Novel => pos_novel += 1,
            Verdict::Familiar => {}
        }
    }
    let pos_n = scores.len();
    let query_ms = t.elapsed().as_secs_f64() * 1e3 / pos_n.max(1) as f64;

    // Negatives: held-out memories, untouched. Caveat: in a brain full of
    // recurring work, some held-out items are GENUINE near-duplicates of
    // enrolled ones. Quantify that label noise independently of the engine:
    // token-set Jaccard vs every enrolled memory; >= 0.5 means the "novel"
    // label is wrong, and the item is excluded from the clean-negative AUC.
    let token_set = |s: &str| -> std::collections::HashSet<String> {
        s.split_whitespace()
            .map(|t| t.to_lowercase())
            .filter(|t| t.len() >= 3)
            .collect()
    };
    let enrolled_sets: Vec<std::collections::HashSet<String>> =
        known.iter().map(|(_, c)| token_set(c)).collect();

    let mut neg_flagged: Vec<(String, String, f64)> = Vec::new(); // (held_id, matched_id, fam)
    let mut label_noise = 0usize;
    let mut noisy: Vec<bool> = Vec::new();
    for (id, content) in &held_out {
        let hs = token_set(content);
        let max_jaccard = enrolled_sets
            .iter()
            .map(|es| {
                let inter = hs.intersection(es).count() as f64;
                let union = (hs.len() + es.len()) as f64 - inter;
                if union > 0.0 { inter / union } else { 0.0 }
            })
            .fold(0.0f64, f64::max);
        let is_noise = max_jaccard >= 0.5;
        if is_noise {
            label_noise += 1;
        }
        noisy.push(is_noise);
        let r = engine.recognize(content)?;
        scores.push((r.familiarity, r.odds_of_old, false));
        if let Verdict::Recognized { memory_id } = &r.verdict {
            neg_flagged.push((id.clone(), memory_id.clone(), r.familiarity));
        }
    }
    let neg_n = scores.len() - pos_n;

    // AUC via rank statistic (ties half credit) over a chosen scalar.
    let auc_over = |pick: &dyn Fn(&(f64, f64, bool)) -> f64| -> f64 {
        let mut num = 0.0f64;
        for p in scores.iter().filter(|s| s.2) {
            for n in scores.iter().filter(|s| !s.2) {
                let (a, b) = (pick(p), pick(n));
                num += if a > b {
                    1.0
                } else if a == b {
                    0.5
                } else {
                    0.0
                };
            }
        }
        num / (pos_n as f64 * neg_n as f64).max(1.0)
    };
    let auc = auc_over(&|s| s.0);
    let auc_odds = auc_over(&|s| s.1);

    // Clean AUC: exclude label-noise negatives (true near-dupes).
    let clean: Vec<&(f64, f64, bool)> = scores
        .iter()
        .enumerate()
        .filter(|(i, s)| s.2 || !noisy[*i - pos_n])
        .map(|(_, s)| s)
        .collect();
    let clean_neg = clean.iter().filter(|s| !s.2).count();
    let mut clean_num = 0.0f64;
    for p in clean.iter().filter(|s| s.2) {
        for n in clean.iter().filter(|s| !s.2) {
            clean_num += if p.0 > n.0 {
                1.0
            } else if p.0 == n.0 {
                0.5
            } else {
                0.0
            };
        }
    }
    let auc_clean = clean_num / (pos_n as f64 * clean_neg as f64).max(1.0);

    println!("== recognition replay ==");
    println!("enrolled:              {}", known.len());
    println!("positives (degraded):  {pos_n}");
    println!("negatives (held-out):  {neg_n}");
    println!("AUC(familiarity):      {auc:.4}");
    println!("AUC(odds_of_old):      {auc_odds:.4}");
    println!(
        "label-noise negatives: {label_noise} / {neg_n} (Jaccard >= 0.5 vs enrolled — 'novel' label is wrong)"
    );
    println!(
        "AUC(clean negatives):  {auc_clean:.4}  ({} clean negatives)",
        neg_n - label_noise
    );
    println!(
        "positives Recognized:  {} ({:.1}%), correct trace {} ({:.1}% of recognized)",
        pos_recognized,
        100.0 * pos_recognized as f64 / pos_n.max(1) as f64,
        pos_correct_trace,
        100.0 * pos_correct_trace as f64 / pos_recognized.max(1) as f64
    );
    println!(
        "positives judged Novel: {} ({:.1}%)  <- misses",
        pos_novel,
        100.0 * pos_novel as f64 / pos_n.max(1) as f64
    );
    println!(
        "negatives Recognized:  {} ({:.1}%)  <- inspect: may be true near-dupes",
        neg_flagged.len(),
        100.0 * neg_flagged.len() as f64 / neg_n.max(1) as f64
    );
    for (held, matched, fam) in neg_flagged.iter().take(12) {
        println!("  held-out {held} locked to {matched} (fam {fam:.3})");
    }
    println!("query latency:         {query_ms:.3} ms/query (in-memory store)");
    Ok(())
}
