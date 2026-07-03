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
use spectral_recognition::eval::{
    degrade, max_jaccard, roc_auc, split_9010, token_set, LABEL_NOISE_JACCARD,
};
use spectral_recognition::{
    InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine, Verdict,
};

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

    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt =
        conn.prepare("SELECT id, content FROM memories WHERE LENGTH(content) >= 60 ORDER BY id")?;
    let memories: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let memories: Vec<_> = memories.into_iter().take(limit).collect();

    let (known, held_out) = split_9010(&memories);
    eprintln!(
        "memories={} enrolled={} held_out={}",
        memories.len(),
        known.len(),
        held_out.len()
    );

    let mut cfg = RecognitionConfig::default();
    // Operating-point sweep knobs (default = shipped thresholds).
    let env_f64 = |k: &str| std::env::var(k).ok().and_then(|s| s.parse::<f64>().ok());
    if let Some(v) = env_f64("SPECTRAL_REC_COVERAGE") {
        cfg.score.recognize_coverage = v;
    }
    if let Some(v) = env_f64("SPECTRAL_REC_MARGIN") {
        cfg.score.recognize_margin = v;
    }
    let mut engine = RecognitionEngine::new(InMemoryRecognitionStore::default(), cfg.clone());
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
    // Lock-rate diagnostics: for positives that do NOT lock (Familiar/Novel),
    // which Recognized gate failed? A positive locks only if coverage,
    // min-score, AND the margin-over-runner-up all hold. Counting each gate's
    // failures (and margin-only failures) shows whether the 35% non-locks are
    // a threshold that could safely move or the anti-flap rule doing its job
    // on the brain's ~29% near-duplicates.
    let (mut g_cov, mut g_score, mut g_margin, mut g_margin_only, mut g_notrace) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
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
            Verdict::Novel | Verdict::Familiar => {
                if matches!(r.verdict, Verdict::Novel) {
                    pos_novel += 1;
                }
                match r.traces.first() {
                    None => g_notrace += 1,
                    Some(best) => {
                        let runner = r.traces.get(1).map(|t| t.score).unwrap_or(0.0);
                        let cov_ok = best.coverage >= cfg.score.recognize_coverage;
                        let score_ok = best.score >= cfg.score.recognize_min_score;
                        let margin_ok = best.score >= runner * cfg.score.recognize_margin;
                        if !cov_ok {
                            g_cov += 1;
                        }
                        if !score_ok {
                            g_score += 1;
                        }
                        if !margin_ok {
                            g_margin += 1;
                        }
                        if cov_ok && score_ok && !margin_ok {
                            g_margin_only += 1;
                        }
                    }
                }
            }
        }
    }
    let pos_n = scores.len();
    let query_ms = t.elapsed().as_secs_f64() * 1e3 / pos_n.max(1) as f64;

    // Negatives: held-out memories, untouched. Caveat: in a brain full of
    // recurring work, some held-out items are GENUINE near-duplicates of
    // enrolled ones. Quantify that label noise independently of the engine:
    // token-set Jaccard vs every enrolled memory; >= 0.5 means the "novel"
    // label is wrong, and the item is excluded from the clean-negative AUC.
    let enrolled_sets: Vec<std::collections::HashSet<String>> =
        known.iter().map(|(_, c)| token_set(c)).collect();

    let mut neg_flagged: Vec<(String, String, f64)> = Vec::new(); // (held_id, matched_id, fam)
    let mut label_noise = 0usize;
    // False locks on CLEAN negatives (the real precision cost of moving the
    // operating point) vs on true near-dupes (which locking is arguably right).
    let mut clean_false_locks = 0usize;
    let mut clean_false_locks_excluded = 0usize;
    let mut noisy: Vec<bool> = Vec::new();
    for (id, content) in &held_out {
        let hs = token_set(content);
        let is_noise = max_jaccard(&hs, &enrolled_sets) >= LABEL_NOISE_JACCARD;
        if is_noise {
            label_noise += 1;
        }
        noisy.push(is_noise);
        let r = engine.recognize(content)?;
        scores.push((r.familiarity, r.odds_of_old, false));
        if let Verdict::Recognized { memory_id } = &r.verdict {
            neg_flagged.push((id.clone(), memory_id.clone(), r.familiarity));
            if is_noise {
                clean_false_locks_excluded += 1;
            } else {
                clean_false_locks += 1;
            }
        }
    }
    let neg_n = scores.len() - pos_n;

    // AUC via rank statistic (ties half credit) over each candidate scalar.
    let auc = roc_auc(&scores.iter().map(|s| (s.0, s.2)).collect::<Vec<_>>());
    let auc_odds = roc_auc(&scores.iter().map(|s| (s.1, s.2)).collect::<Vec<_>>());

    // Clean AUC: exclude label-noise negatives (true near-dupes).
    let clean: Vec<(f64, bool)> = scores
        .iter()
        .enumerate()
        .filter(|(i, s)| s.2 || !noisy[*i - pos_n])
        .map(|(_, s)| (s.0, s.2))
        .collect();
    let clean_neg = clean.iter().filter(|s| !s.1).count();
    let auc_clean = roc_auc(&clean);

    println!("== recognition replay ==");
    println!("enrolled:              {}", known.len());
    println!("positives (degraded):  {pos_n}");
    println!("negatives (held-out):  {neg_n}");
    println!("AUC(familiarity):      {auc:.4}");
    println!("AUC(odds_of_old):      {auc_odds:.4}");
    println!(
        "label-noise negatives: {label_noise} / {neg_n} (Jaccard >= 0.5 vs enrolled — 'novel' label is wrong)"
    );
    println!("AUC(clean negatives):  {auc_clean:.4}  ({clean_neg} clean negatives)");
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
    let non_lock = pos_n - pos_recognized;
    println!(
        "non-lock gate failures ({non_lock} non-locking positives): coverage<{:.2} {g_cov}, score<{:.1} {g_score}, margin<{:.1}x {g_margin} (margin-only {g_margin_only}), no-trace {g_notrace}",
        cfg.score.recognize_coverage, cfg.score.recognize_min_score, cfg.score.recognize_margin
    );
    println!(
        "negatives Recognized:  {} ({:.1}%)  <- {clean_false_locks} on CLEAN negatives, {clean_false_locks_excluded} on true near-dupes",
        neg_flagged.len(),
        100.0 * neg_flagged.len() as f64 / neg_n.max(1) as f64
    );
    for (held, matched, fam) in neg_flagged.iter().take(12) {
        println!("  held-out {held} locked to {matched} (fam {fam:.3})");
    }
    println!("query latency:         {query_ms:.3} ms/query (in-memory store)");
    Ok(())
}
