//! R42 diagnostic — WHICH gate turns an exact re-encounter into `Familiar`?
//!
//! `recognition_e2e` established that on the real brain 16.3% of probes made
//! of a memory's own content come back `Familiar` rather than `Recognized`,
//! with top-1 identity still correct 95.3% of the time. That is a lost
//! verdict, not a lost memory, and it is invisible to the aggregate numbers.
//!
//! The `Recognized` gate is a conjunction of four conditions (ScoreConfig):
//!   coverage    >= recognize_coverage        (0.35)
//!   score       >= recognize_min_score       (3.0)
//!   familiarity >= recognize_min_familiarity (0.60)
//!   lead        >= recognize_margin          (1.5x the runner-up's score)
//! This reports, for every probe that fails, which condition(s) failed and by
//! how much — and whether the runner-up is a near-duplicate of the target
//! (shingle containment), which is the "same-template family" hypothesis.
//!
//! Read-only, in-memory index, $0, seconds.
//!
//! usage: recognition_gate_diagnostic <memory.db> [sample_n]

use spectral_recognition::{
    minhash, InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine, Verdict,
};
use std::collections::HashMap;

fn pct(k: usize, n: usize) -> f64 {
    100.0 * k as f64 / n.max(1) as f64
}

fn median(mut xs: Vec<f64>) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs[xs.len() / 2]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let db = args
        .get(1)
        .ok_or("usage: recognition_gate_diagnostic <memory.db> [sample_n]")?;
    let sample_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);

    let conn = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let mut stmt = conn.prepare(
        "SELECT id, content FROM memories
         WHERE content IS NOT NULL AND TRIM(content) <> '' ORDER BY id",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;

    let cfg = RecognitionConfig::default();
    let shingle = cfg.minhash.shingle;
    let sc = cfg.score.clone();
    let mut engine = RecognitionEngine::new(InMemoryRecognitionStore::default(), cfg);
    let by_id: HashMap<&str, &str> = rows
        .iter()
        .map(|(i, c)| (i.as_str(), c.as_str()))
        .collect();
    for (id, content) in &rows {
        engine.enroll(id, content)?;
    }

    // Deterministic stride sample, so a re-run measures the same memories.
    let stride = (rows.len() / sample_n.max(1)).max(1);
    let sample: Vec<&(String, String)> = rows.iter().step_by(stride).take(sample_n).collect();

    println!("R42 gate diagnostic — {} enrolled, {} probed (own content)", rows.len(), sample.len());
    println!(
        "  gate: coverage>={:.2} AND score>={:.1} AND familiarity>={:.2} AND lead>={:.1}x",
        sc.recognize_coverage, sc.recognize_min_score, sc.recognize_min_familiarity, sc.recognize_margin
    );

    let (mut recognized, mut familiar, mut novel) = (0usize, 0usize, 0usize);
    let mut fail_cov = 0usize;
    let mut fail_score = 0usize;
    let mut fail_fam = 0usize;
    let mut fail_lead = 0usize;
    let mut fail_lead_only = 0usize;
    let mut top1_wrong = 0usize;
    let mut near_dup_runner_up = 0usize;
    let mut identical_runner_up = 0usize;
    let mut lead_ratios = Vec::new();
    let mut containments = Vec::new();
    let mut examples: Vec<String> = Vec::new();

    for (id, content) in &sample {
        let r = engine.recognize(content)?;
        match &r.verdict {
            Verdict::Recognized { .. } => {
                recognized += 1;
                continue;
            }
            Verdict::Familiar => familiar += 1,
            Verdict::Novel => novel += 1,
        }
        let Some(best) = r.traces.first() else { continue };
        if &best.memory_id != id {
            top1_wrong += 1;
        }
        let runner = r.traces.get(1);
        let runner_score = runner.map(|t| t.score).unwrap_or(0.0);
        let lead = if runner_score > 0.0 {
            best.score / runner_score
        } else {
            f64::INFINITY
        };
        lead_ratios.push(lead.min(99.0));

        let c_ok = best.coverage >= sc.recognize_coverage;
        let s_ok = best.score >= sc.recognize_min_score;
        let f_ok = r.familiarity >= sc.recognize_min_familiarity;
        let l_ok = best.score >= runner_score * sc.recognize_margin;
        if !c_ok {
            fail_cov += 1;
        }
        if !s_ok {
            fail_score += 1;
        }
        if !f_ok {
            fail_fam += 1;
        }
        if !l_ok {
            fail_lead += 1;
        }
        if !l_ok && c_ok && s_ok && f_ok {
            fail_lead_only += 1;
        }

        // Is the runner-up a near-duplicate of the target? (template family)
        if let Some(ru) = runner {
            if let (Some(a), Some(b)) = (by_id.get(id.as_str()), by_id.get(ru.memory_id.as_str())) {
                let sa = minhash::shingle_set(a, shingle);
                let sb = minhash::shingle_set(b, shingle);
                let cont = minhash::containment(&sa, &sb);
                containments.push(cont);
                if cont >= 0.5 {
                    near_dup_runner_up += 1;
                }
                if a == b {
                    identical_runner_up += 1;
                }
                if examples.len() < 5 && !l_ok && cont >= 0.5 {
                    examples.push(format!(
                        "    lead {:.2}x, containment {:.2}\n      target : {}\n      runner : {}",
                        lead,
                        cont,
                        a.chars().take(90).collect::<String>().replace('\n', " "),
                        b.chars().take(90).collect::<String>().replace('\n', " ")
                    ));
                }
            }
        }
    }

    let n = sample.len();
    let missed = familiar + novel;
    println!(
        "\n  Recognized {:5.1}%   Familiar {:5.1}%   Novel {:5.1}%",
        pct(recognized, n),
        pct(familiar, n),
        pct(novel, n)
    );
    if missed == 0 {
        println!("  nothing missed — no gate to diagnose");
        return Ok(());
    }
    println!("\n  Of the {missed} probes that did NOT reach Recognized:");
    println!("    top-1 identity WRONG        : {:3} ({:5.1}%)", top1_wrong, pct(top1_wrong, missed));
    println!("    failed coverage gate        : {:3} ({:5.1}%)", fail_cov, pct(fail_cov, missed));
    println!("    failed score gate           : {:3} ({:5.1}%)", fail_score, pct(fail_score, missed));
    println!("    failed familiarity gate     : {:3} ({:5.1}%)", fail_fam, pct(fail_fam, missed));
    println!("    failed LEAD-MARGIN gate     : {:3} ({:5.1}%)", fail_lead, pct(fail_lead, missed));
    println!("    failed LEAD MARGIN *ONLY*   : {:3} ({:5.1}%)  <- recoverable without touching any threshold", fail_lead_only, pct(fail_lead_only, missed));
    println!(
        "    runner-up is a near-duplicate (containment >= 0.50): {:3} ({:5.1}%)",
        near_dup_runner_up,
        pct(near_dup_runner_up, missed)
    );
    println!(
        "    runner-up content is BYTE-IDENTICAL to the target  : {:3} ({:5.1}%)  <- a tie no content engine can break",
        identical_runner_up,
        pct(identical_runner_up, missed)
    );
    println!(
        "    median lead ratio {:.2}x (need {:.1}x); median runner-up containment {:.2}",
        median(lead_ratios),
        sc.recognize_margin,
        median(containments)
    );
    if !examples.is_empty() {
        println!("\n  Examples of lead-margin failures against a near-duplicate:");
        for e in &examples {
            println!("{e}");
        }
    }
    Ok(())
}
