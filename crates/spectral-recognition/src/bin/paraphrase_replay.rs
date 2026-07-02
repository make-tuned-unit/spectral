//! Paraphrase-family recognition replay: the HARD test. Positives are
//! Haiku paraphrases (same facts, different words) of enrolled memories;
//! negatives are held-out memories with label-noise exclusion (Jaccard
//! near-dupes of enrolled don't count as negatives).
//!
//! Usage: paraphrase_replay --db <memory.db> --paraphrases <paraphrases.json>

use anyhow::{Context, Result};
use spectral_recognition::{
    InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine, Verdict,
};

fn hash_id(id: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(id.as_bytes());
    u64::from_be_bytes(d[..8].try_into().unwrap())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let db = args
        .iter()
        .position(|a| a == "--db")
        .and_then(|i| args.get(i + 1))
        .context("--db required")?;
    let para_path = args
        .iter()
        .position(|a| a == "--paraphrases")
        .and_then(|i| args.get(i + 1))
        .context("--paraphrases required")?;

    let paraphrases: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(para_path)?)?;

    let conn = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut stmt =
        conn.prepare("SELECT id, content FROM memories WHERE LENGTH(content) >= 60 ORDER BY id")?;
    let memories: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<_, _>>()?;

    let (known, held_out): (Vec<_>, Vec<_>) =
        memories.iter().partition(|(id, _)| hash_id(id) % 10 != 0);

    let mut engine = RecognitionEngine::new(
        InMemoryRecognitionStore::default(),
        RecognitionConfig::default(),
    );
    for (id, content) in &known {
        engine.enroll(id, content)?;
    }

    // Positives: paraphrases of ENROLLED memories.
    let mut scores: Vec<(f64, bool)> = Vec::new();
    let (mut recognized, mut correct_trace, mut familiar, mut novel) = (0, 0, 0, 0);
    let enrolled_ids: std::collections::HashSet<&str> =
        known.iter().map(|(id, _)| id.as_str()).collect();
    for (id, para) in &paraphrases {
        if !enrolled_ids.contains(id.as_str()) {
            continue;
        }
        let r = engine.recognize(para)?;
        scores.push((r.familiarity, true));
        match &r.verdict {
            Verdict::Recognized { memory_id } => {
                recognized += 1;
                if memory_id == id {
                    correct_trace += 1;
                }
            }
            Verdict::Familiar => familiar += 1,
            Verdict::Novel => novel += 1,
        }
    }
    let pos_n = scores.len();

    // Negatives with label-noise exclusion (same as replay.rs).
    let token_set = |s: &str| -> std::collections::HashSet<String> {
        s.split_whitespace()
            .map(|t| t.to_lowercase())
            .filter(|t| t.len() >= 3)
            .collect()
    };
    let enrolled_sets: Vec<std::collections::HashSet<String>> =
        known.iter().map(|(_, c)| token_set(c)).collect();
    let mut clean_neg = 0usize;
    for (_id, content) in &held_out {
        let hs = token_set(content);
        let max_j = enrolled_sets
            .iter()
            .map(|es| {
                let i = hs.intersection(es).count() as f64;
                let u = (hs.len() + es.len()) as f64 - i;
                if u > 0.0 { i / u } else { 0.0 }
            })
            .fold(0.0f64, f64::max);
        if max_j >= 0.5 {
            continue; // true near-dupe — not a valid negative
        }
        let r = engine.recognize(content)?;
        scores.push((r.familiarity, false));
        clean_neg += 1;
    }

    let mut auc_num = 0.0f64;
    for p in scores.iter().filter(|s| s.1) {
        for n in scores.iter().filter(|s| !s.1) {
            auc_num += if p.0 > n.0 { 1.0 } else if p.0 == n.0 { 0.5 } else { 0.0 };
        }
    }
    let auc = auc_num / (pos_n as f64 * clean_neg as f64).max(1.0);

    println!("== paraphrase recognition replay ==");
    println!("enrolled:               {}", known.len());
    println!("paraphrase positives:   {pos_n}");
    println!("clean negatives:        {clean_neg}");
    println!("AUC(familiarity):       {auc:.4}");
    println!(
        "positives: Recognized {} ({:.1}%, correct trace {:.1}%), Familiar {} ({:.1}%), Novel {} ({:.1}%)",
        recognized,
        100.0 * recognized as f64 / pos_n.max(1) as f64,
        100.0 * correct_trace as f64 / recognized.max(1) as f64,
        familiar,
        100.0 * familiar as f64 / pos_n.max(1) as f64,
        novel,
        100.0 * novel as f64 / pos_n.max(1) as f64
    );

    // Failure analysis: dump the paraphrase-positives judged Novel.
    let mut miss_report = String::new();
    for (id, para) in &paraphrases {
        if !enrolled_ids.contains(id.as_str()) {
            continue;
        }
        let r = engine.recognize(para)?;
        if r.verdict == Verdict::Novel {
            let orig = known.iter().find(|(i, _)| i == id).map(|(_, c)| c.as_str()).unwrap_or("");
            miss_report.push_str(&format!(
                "--- MISS {id} (fam {:.3}, peaks {})\nORIG: {}\nPARA: {}\n\n",
                r.familiarity,
                r.stimulus_peaks,
                &orig[..orig.len().min(200)],
                &para[..para.len().min(200)]
            ));
        }
    }
    // The dump contains real memory content — keep it OUT of the repo tree.
    let dump = std::env::var("HOME").unwrap_or_default() + "/spectral-local-bench/paraphrase-misses.txt";
    std::fs::write(&dump, &miss_report)?;
    println!("miss dump:              {dump}");
    Ok(())
}
