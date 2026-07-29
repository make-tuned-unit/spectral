//! public_bench — the pre-registered PUBLIC recognition benchmark harness.
//!
//! Implements the experimental design committed (before any measurement) in
//! `docs/internal/recognition-public-benchmark-prereg-2026-07-28.md`. Three
//! regimes on public data, identical splits and identical `eval::roc_auc`
//! scoring for every system:
//!
//! - **R1 lexical** — LongMemEval-S user turns ≥ 60 chars (deduped by session
//!   id, deterministic derivation order, `--r1-limit` take-first-N cap).
//!   Enroll = 90% by `hash_id % 10`; positives = enrolled with 30%
//!   deterministic token dropout; negatives = held-out turns. The label-noise
//!   mask (token Jaccard ≥ 0.5 vs any enrolled) is applied at SPLIT
//!   CONSTRUCTION, so every system — including out-of-band baselines — scores
//!   the identical clean instances.
//! - **R2 semantic** — MRPC test, label=1 pairs. Pairs are split in half by
//!   `hash_id("mrpc:<line>") % 2`: the enrolled half contributes (enroll A,
//!   probe B, positive); the other half's B-sides are probed WITHOUT their
//!   A-side enrolled (negative).
//! - **R3 adversarial** — PAWS-Wiki labeled test. ALL A-sides enrolled; every
//!   B-side probed; label=1 → positive, label=0 (high overlap, different
//!   meaning) → negative. A familiar verdict on label=0 is a false positive.
//!
//! In-binary systems: peak-pair engine (query mode) and MinHash-128 (the same
//! implementation as `classical_baselines.rs` — the private matrix's lexical
//! champion). The BGE-small embedding baseline runs OUT-OF-BAND in python and
//! MUST consume `--export-splits` output rather than re-deriving splits.
//!
//! `--export-splits <dir>` writes, per regime (deterministic bytes, eval
//! order):
//! - `<dir>/r{1,2,3}_probes.jsonl` — one `{"regime","probe","label"}` per
//!   evaluation instance, in the exact order this binary scores them.
//! - `<dir>/r{1,2,3}_enroll.jsonl` — a single `{"regime","enroll":[...]}`
//!   record listing every enrolled text in enrollment order.
//!
//! Usage:
//!   public_bench [--longmemeval <longmemeval_s.json>] [--mrpc <mrpc.jsonl>]
//!                [--paws <paws.jsonl>] [--r1-limit N] [--json <out.json>]
//!                [--export-splits <dir>]

use anyhow::{bail, Context, Result};
use spectral_recognition::eval::{
    degrade, hash_id, max_jaccard, roc_auc, split_9010, token_set, LABEL_NOISE_JACCARD,
};
use spectral_recognition::{
    InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine, Verdict,
};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

// ── MinHash-128 baseline (identical to classical_baselines.rs) ───────────

const K: usize = 128;

fn hash64(s: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(s.as_bytes());
    u64::from_be_bytes(d[..8].try_into().unwrap())
}

/// splitmix64 — deterministic constants for the MinHash universal-hash family.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Lowercased whitespace tokens, length ≥ 3.
fn tokens(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() >= 3)
        .collect()
}

fn minhash_sig(toks: &[String], a: &[u64], b: &[u64]) -> [u64; K] {
    let mut sig = [u64::MAX; K];
    let mut seen: HashMap<&str, u64> = HashMap::new();
    for t in toks {
        seen.entry(t).or_insert_with(|| hash64(t));
    }
    for base in seen.values() {
        for i in 0..K {
            let h = base.wrapping_mul(a[i]).wrapping_add(b[i]);
            if h < sig[i] {
                sig[i] = h;
            }
        }
    }
    sig
}

fn minhash_sim(x: &[u64; K], y: &[u64; K]) -> f64 {
    let eq = x.iter().zip(y).filter(|(p, q)| p == q).count();
    eq as f64 / K as f64
}

// ── Regime construction (the byte-identical instance sets) ───────────────

/// One regime's exact evaluation instances. `probes` is in evaluation order —
/// the same order `--export-splits` writes and both systems score.
struct RegimeData {
    name: &'static str,
    title: &'static str,
    /// (memory_id, content) in enrollment order.
    enroll: Vec<(String, String)>,
    /// (probe_text, is_positive) in evaluation order.
    probes: Vec<(String, bool)>,
    /// R1 only: held-out negatives excluded by the label-noise mask.
    label_noise_excluded: usize,
}

#[derive(serde::Deserialize)]
struct LmeQuestion {
    haystack_session_ids: Vec<String>,
    haystack_sessions: Vec<Vec<LmeTurn>>,
}

#[derive(serde::Deserialize)]
struct LmeTurn {
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: String,
}

fn build_r1(path: &Path, limit: usize) -> Result<RegimeData> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let questions: Vec<LmeQuestion> =
        serde_json::from_str(&text).context("parsing LongMemEval-S json")?;
    drop(text);

    // Deterministic derivation: question order → session order → turn order,
    // sessions deduped by id, take-first-N under the cap.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut turns: Vec<(String, String)> = Vec::new();
    'outer: for q in &questions {
        for (sid, sess) in q.haystack_session_ids.iter().zip(&q.haystack_sessions) {
            if !seen.insert(sid) {
                continue;
            }
            for (ti, t) in sess.iter().enumerate() {
                if t.role == "user" && t.content.chars().count() >= 60 {
                    turns.push((format!("{sid}:{ti}"), t.content.clone()));
                    if turns.len() >= limit {
                        break 'outer;
                    }
                }
            }
        }
    }
    if turns.is_empty() {
        bail!("no user turns ≥ 60 chars found in {}", path.display());
    }

    let (known, held_out) = split_9010(&turns);
    let enrolled_sets: Vec<_> = known.iter().map(|(_, c)| token_set(c)).collect();

    let mut probes = Vec::new();
    for (id, content) in &known {
        let stim = degrade(content, id, 30);
        if stim.split_whitespace().count() < 5 {
            continue;
        }
        probes.push((stim, true));
    }
    let mut excluded = 0usize;
    for (_, content) in &held_out {
        if max_jaccard(&token_set(content), &enrolled_sets) >= LABEL_NOISE_JACCARD {
            excluded += 1; // genuine near-dup of an enrolled turn: "novel" label is wrong
            continue;
        }
        probes.push((content.clone(), false));
    }

    Ok(RegimeData {
        name: "r1",
        title: "R1 lexical (LongMemEval-S)",
        enroll: known,
        probes,
        label_noise_excluded: excluded,
    })
}

#[derive(serde::Deserialize)]
struct PairRow {
    s1: String,
    s2: String,
    label: u8,
}

fn read_pairs(path: &Path) -> Result<Vec<(usize, PairRow)>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: PairRow =
            serde_json::from_str(line).with_context(|| format!("{}:{}", path.display(), i + 1))?;
        out.push((i, row));
    }
    Ok(out)
}

fn build_r2(path: &Path) -> Result<RegimeData> {
    let mut enroll = Vec::new();
    let mut probes = Vec::new();
    for (i, row) in read_pairs(path)? {
        if row.label != 1 {
            continue; // genuine paraphrases only
        }
        let pid = format!("mrpc:{i}");
        if hash_id(&pid).is_multiple_of(2) {
            // Enrolled half: A enters the store, B is a true re-encounter.
            enroll.push((pid, row.s1));
            probes.push((row.s2, true));
        } else {
            // Held half: B probed against a store WITHOUT its A-side.
            probes.push((row.s2, false));
        }
    }
    if enroll.is_empty() {
        bail!("no label=1 MRPC pairs found in {}", path.display());
    }
    Ok(RegimeData {
        name: "r2",
        title: "R2 semantic (MRPC label=1)",
        enroll,
        probes,
        label_noise_excluded: 0,
    })
}

fn build_r3(path: &Path) -> Result<RegimeData> {
    let mut enroll = Vec::new();
    let mut probes = Vec::new();
    for (i, row) in read_pairs(path)? {
        enroll.push((format!("paws:{i}"), row.s1));
        probes.push((row.s2, row.label == 1));
    }
    if enroll.is_empty() {
        bail!("no PAWS pairs found in {}", path.display());
    }
    Ok(RegimeData {
        name: "r3",
        title: "R3 adversarial (PAWS-Wiki)",
        enroll,
        probes,
        label_noise_excluded: 0,
    })
}

// ── Systems ──────────────────────────────────────────────────────────────

struct EngineResult {
    auc: f64,
    pos_novel: usize,
    neg_familiar: usize,
    enroll_ms: f64,
    query_ms: f64,
}

fn run_engine(rd: &RegimeData) -> Result<EngineResult> {
    let mut engine = RecognitionEngine::new(
        InMemoryRecognitionStore::default(),
        RecognitionConfig::default(),
    );
    let t = std::time::Instant::now();
    for (id, content) in &rd.enroll {
        engine.enroll(id, content)?;
    }
    let enroll_ms = t.elapsed().as_secs_f64() * 1e3;

    let mut scores = Vec::with_capacity(rd.probes.len());
    let (mut pos_novel, mut neg_familiar) = (0usize, 0usize);
    let t = std::time::Instant::now();
    for (probe, is_pos) in &rd.probes {
        let r = engine.recognize(probe)?;
        scores.push((r.familiarity, *is_pos));
        match (&r.verdict, is_pos) {
            (Verdict::Novel, true) => pos_novel += 1,
            (Verdict::Familiar | Verdict::Recognized { .. }, false) => neg_familiar += 1,
            _ => {}
        }
    }
    let query_ms = t.elapsed().as_secs_f64() * 1e3 / rd.probes.len().max(1) as f64;
    Ok(EngineResult {
        auc: roc_auc(&scores),
        pos_novel,
        neg_familiar,
        enroll_ms,
        query_ms,
    })
}

fn run_minhash(rd: &RegimeData) -> (f64, f64) {
    let a: Vec<u64> = (0..K).map(|i| splitmix64(i as u64 * 2 + 1) | 1).collect();
    let b: Vec<u64> = (0..K).map(|i| splitmix64(i as u64 * 2 + 2)).collect();
    let enrolled: Vec<[u64; K]> = rd
        .enroll
        .iter()
        .map(|(_, c)| minhash_sig(&tokens(c), &a, &b))
        .collect();
    let mut scores = Vec::with_capacity(rd.probes.len());
    let t = std::time::Instant::now();
    for (probe, is_pos) in &rd.probes {
        let sig = minhash_sig(&tokens(probe), &a, &b);
        let best = enrolled
            .iter()
            .map(|e| minhash_sim(&sig, e))
            .fold(0.0f64, f64::max);
        scores.push((best, *is_pos));
    }
    let query_ms = t.elapsed().as_secs_f64() * 1e3 / rd.probes.len().max(1) as f64;
    (roc_auc(&scores), query_ms)
}

// ── Split export (the out-of-band contract) ──────────────────────────────

#[derive(serde::Serialize)]
struct ProbeRec<'a> {
    regime: &'a str,
    probe: &'a str,
    label: bool,
}

#[derive(serde::Serialize)]
struct EnrollRec<'a> {
    regime: &'a str,
    enroll: &'a [&'a str],
}

fn export_splits(dir: &Path, rd: &RegimeData) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let probes_path = dir.join(format!("{}_probes.jsonl", rd.name));
    let mut pf = BufWriter::new(File::create(&probes_path)?);
    for (probe, label) in &rd.probes {
        serde_json::to_writer(
            &mut pf,
            &ProbeRec {
                regime: rd.name,
                probe,
                label: *label,
            },
        )?;
        pf.write_all(b"\n")?;
    }
    pf.flush()?;

    let enroll_path = dir.join(format!("{}_enroll.jsonl", rd.name));
    let mut ef = BufWriter::new(File::create(&enroll_path)?);
    let texts: Vec<&str> = rd.enroll.iter().map(|(_, c)| c.as_str()).collect();
    serde_json::to_writer(
        &mut ef,
        &EnrollRec {
            regime: rd.name,
            enroll: &texts,
        },
    )?;
    ef.write_all(b"\n")?;
    ef.flush()?;
    eprintln!(
        "exported {} ({} probes) and {} ({} enrolled)",
        probes_path.display(),
        rd.probes.len(),
        enroll_path.display(),
        rd.enroll.len()
    );
    Ok(())
}

// ── Main ─────────────────────────────────────────────────────────────────

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let longmemeval = arg_value(&args, "--longmemeval").map(PathBuf::from);
    let mrpc = arg_value(&args, "--mrpc").map(PathBuf::from);
    let paws = arg_value(&args, "--paws").map(PathBuf::from);
    let r1_limit: usize = arg_value(&args, "--r1-limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    let json_out = arg_value(&args, "--json").map(PathBuf::from);
    let export_dir = arg_value(&args, "--export-splits").map(PathBuf::from);
    if longmemeval.is_none() && mrpc.is_none() && paws.is_none() {
        bail!(
            "usage: public_bench [--longmemeval <longmemeval_s.json>] [--mrpc <mrpc.jsonl>] \
             [--paws <paws.jsonl>] [--r1-limit N] [--json <out.json>] [--export-splits <dir>]"
        );
    }

    let mut regimes: Vec<RegimeData> = Vec::new();
    if let Some(p) = &longmemeval {
        regimes.push(build_r1(p, r1_limit)?);
    }
    if let Some(p) = &mrpc {
        regimes.push(build_r2(p)?);
    }
    if let Some(p) = &paws {
        regimes.push(build_r3(p)?);
    }

    if let Some(dir) = &export_dir {
        for rd in &regimes {
            export_splits(dir, rd)?;
        }
    }

    println!("== public recognition benchmark — pre-registered 2026-07-28 ==");
    println!("(design: docs/internal/recognition-public-benchmark-prereg-2026-07-28.md)");
    if longmemeval.is_some() {
        println!("r1-limit: {r1_limit} (take-first-N in deterministic derivation order)");
    }
    println!();
    println!(
        "{:28} {:>7} {:>7} {:>7}   {:>10}   {:>14}",
        "regime", "enroll", "pos", "neg", "engine-AUC", "minhash128-AUC"
    );

    let mut json_regimes = Vec::new();
    for rd in &regimes {
        let pos_n = rd.probes.iter().filter(|p| p.1).count();
        let neg_n = rd.probes.len() - pos_n;
        eprintln!(
            "[{}] enroll={} pos={} neg={} label-noise-excluded={}",
            rd.name,
            rd.enroll.len(),
            pos_n,
            neg_n,
            rd.label_noise_excluded
        );
        let eng = run_engine(rd)?;
        let (mh_auc, mh_query_ms) = run_minhash(rd);
        println!(
            "{:28} {:>7} {:>7} {:>7}   {:>10.4}   {:>14.4}",
            rd.title,
            rd.enroll.len(),
            pos_n,
            neg_n,
            eng.auc,
            mh_auc
        );
        println!(
            "  engine verdicts: positives Novel {}/{pos_n} ({:.1}%), negatives non-Novel {}/{neg_n} ({:.1}%)",
            eng.pos_novel,
            100.0 * eng.pos_novel as f64 / pos_n.max(1) as f64,
            eng.neg_familiar,
            100.0 * eng.neg_familiar as f64 / neg_n.max(1) as f64
        );
        println!(
            "  cost: engine enroll {:.0}ms total, query {:.3}ms/probe; minhash query {:.3}ms/probe",
            eng.enroll_ms, eng.query_ms, mh_query_ms
        );
        if rd.label_noise_excluded > 0 {
            println!(
                "  label-noise mask: {} held-out negatives excluded at split construction (Jaccard >= {LABEL_NOISE_JACCARD})",
                rd.label_noise_excluded
            );
        }
        json_regimes.push(serde_json::json!({
            "name": rd.name,
            "title": rd.title,
            "enrolled": rd.enroll.len(),
            "positives": pos_n,
            "negatives": neg_n,
            "label_noise_excluded": rd.label_noise_excluded,
            "engine": {
                "auc": eng.auc,
                "pos_novel": eng.pos_novel,
                "neg_familiar": eng.neg_familiar,
                "enroll_ms_total": eng.enroll_ms,
                "query_ms_avg": eng.query_ms,
            },
            "minhash128": {
                "auc": mh_auc,
                "query_ms_avg": mh_query_ms,
            },
        }));
    }
    println!();
    println!(
        "BGE-small baseline: run out-of-band on --export-splits output (byte-identical instances)."
    );

    if let Some(path) = &json_out {
        let doc = serde_json::json!({
            "benchmark": "recognition-public-prereg-2026-07-28",
            "r1_limit": longmemeval.as_ref().map(|_| r1_limit),
            "regimes": json_regimes,
        });
        std::fs::write(path, serde_json::to_string_pretty(&doc)?)?;
        eprintln!("wrote {}", path.display());
    }
    Ok(())
}
