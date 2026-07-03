//! Classical near-duplicate baselines for recognition — the honest bar.
//!
//! A neural embedding (BGE-small) is the wrong thing to beat on a lexical
//! re-encounter task; the *right* bar is the cheap classical near-dup methods:
//! MinHash (Jaccard estimate), SimHash (Charikar), and BM25. This bin runs all
//! three — plus the peak-pair engine — on the EXACT replay protocol (shared
//! `eval` module: same split, degrade, label-noise mask, rank-statistic AUC),
//! so the only variable is the familiarity scalar. If a 30-line MinHash matches
//! the engine, we say so; the engine's moat is auditability + cost, not a
//! secret accuracy edge.
//!
//! Usage: classical_baselines --db <memory.db> [--limit N]

use anyhow::{Context, Result};
use spectral_recognition::eval::{
    degrade, max_jaccard, roc_auc, split_9010, token_set, LABEL_NOISE_JACCARD,
};
use spectral_recognition::{InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine};
use std::collections::HashMap;

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

const K: usize = 128; // MinHash permutations

/// Lowercased tokens (len >= 3), duplicates kept (BM25 needs term frequency).
fn tokens(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() >= 3)
        .collect()
}

// ── MinHash ──────────────────────────────────────────────────────────
fn minhash_sig(toks: &[String], a: &[u64], b: &[u64]) -> [u64; K] {
    let mut sig = [u64::MAX; K];
    // distinct token base-hashes
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

// ── SimHash (64-bit) ─────────────────────────────────────────────────
fn simhash(toks: &[String]) -> u64 {
    let mut v = [0i32; 64];
    // term-frequency weighted (Charikar)
    let mut tf: HashMap<&str, i32> = HashMap::new();
    for t in toks {
        *tf.entry(t).or_default() += 1;
    }
    for (t, w) in tf {
        let h = hash64(t);
        for (i, slot) in v.iter_mut().enumerate() {
            if (h >> i) & 1 == 1 {
                *slot += w;
            } else {
                *slot -= w;
            }
        }
    }
    let mut fp = 0u64;
    for (i, slot) in v.iter().enumerate() {
        if *slot > 0 {
            fp |= 1 << i;
        }
    }
    fp
}
fn simhash_sim(x: u64, y: u64) -> f64 {
    1.0 - ((x ^ y).count_ones() as f64 / 64.0)
}

// ── BM25 (inverted index over enrolled memories) ─────────────────────
struct Bm25 {
    postings: HashMap<String, Vec<(usize, u32)>>, // term -> [(doc, tf)]
    doc_len: Vec<u32>,
    avg_len: f64,
    n: usize,
}
impl Bm25 {
    fn build(docs: &[Vec<String>]) -> Self {
        let mut postings: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
        let mut doc_len = Vec::with_capacity(docs.len());
        for (d, toks) in docs.iter().enumerate() {
            let mut tf: HashMap<&str, u32> = HashMap::new();
            for t in toks {
                *tf.entry(t).or_default() += 1;
            }
            doc_len.push(toks.len() as u32);
            for (t, c) in tf {
                postings.entry(t.to_string()).or_default().push((d, c));
            }
        }
        let total: u64 = doc_len.iter().map(|&l| l as u64).sum();
        let n = docs.len();
        let avg_len = if n > 0 { total as f64 / n as f64 } else { 0.0 };
        Bm25 {
            postings,
            doc_len,
            avg_len,
            n,
        }
    }
    /// Max BM25 score of the probe against any enrolled doc.
    fn max_score(&self, probe: &[String]) -> f64 {
        const K1: f64 = 1.5;
        const B: f64 = 0.75;
        let mut acc: HashMap<usize, f64> = HashMap::new();
        // distinct probe terms
        let mut qterms: HashMap<&str, ()> = HashMap::new();
        for t in probe {
            qterms.insert(t, ());
        }
        for t in qterms.keys() {
            let Some(post) = self.postings.get(*t) else {
                continue;
            };
            let df = post.len() as f64;
            let idf = (((self.n as f64 - df + 0.5) / (df + 0.5)) + 1.0).ln();
            for &(doc, tf) in post {
                let dl = self.doc_len[doc] as f64;
                let denom = tf as f64 + K1 * (1.0 - B + B * dl / self.avg_len.max(1.0));
                let s = idf * (tf as f64 * (K1 + 1.0)) / denom;
                *acc.entry(doc).or_default() += s;
            }
        }
        acc.values().copied().fold(0.0f64, f64::max)
    }
}

fn clean_auc(scores: &[(f64, bool)], noisy: &[bool], pos_n: usize) -> f64 {
    let clean: Vec<(f64, bool)> = scores
        .iter()
        .enumerate()
        .filter(|(i, s)| s.1 || !noisy[*i - pos_n])
        .map(|(_, s)| *s)
        .collect();
    roc_auc(&clean)
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
    // Optional paraphrase mode: positives are paraphrases of enrolled memories
    // (the SEMANTIC re-encounter test) instead of degraded copies (the lexical
    // near-dup test). This is where peak-pairs should beat raw-token methods.
    let paraphrases: Option<HashMap<String, String>> = args
        .iter()
        .position(|a| a == "--paraphrases")
        .and_then(|i| args.get(i + 1))
        .map(|p| -> Result<HashMap<String, String>> {
            Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?)
        })
        .transpose()?;

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

    // MinHash universal-hash constants (deterministic).
    let a: Vec<u64> = (0..K).map(|i| splitmix64(i as u64 * 2 + 1) | 1).collect();
    let b: Vec<u64> = (0..K).map(|i| splitmix64(i as u64 * 2 + 2)).collect();

    // ── Enroll: engine + classical signatures over the known set ──
    let mut engine = RecognitionEngine::new(
        InMemoryRecognitionStore::default(),
        RecognitionConfig::default(),
    );
    for (id, content) in &known {
        engine.enroll(id, content)?;
    }
    let known_toks: Vec<Vec<String>> = known.iter().map(|(_, c)| tokens(c)).collect();
    let mh: Vec<[u64; K]> = known_toks.iter().map(|t| minhash_sig(t, &a, &b)).collect();
    let sh: Vec<u64> = known_toks.iter().map(|t| simhash(t)).collect();
    let bm25 = Bm25::build(&known_toks);
    let enrolled_sets: Vec<_> = known.iter().map(|(_, c)| token_set(c)).collect();

    // score vectors: (scalar, is_positive) per method
    let (mut s_eng, mut s_mh, mut s_sh, mut s_bm) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());

    // ── Positives: degraded enrolled (lexical) OR paraphrases (semantic) ──
    let mode = if paraphrases.is_some() {
        "paraphrase (semantic)"
    } else {
        "degraded copy (lexical)"
    };
    let mut pos_n = 0usize;
    for (id, content) in &known {
        let stim = match &paraphrases {
            Some(pm) => match pm.get(id) {
                Some(p) => p.clone(),
                None => continue, // only enrolled memories with a paraphrase
            },
            None => degrade(content, id, 30),
        };
        if stim.split_whitespace().count() < 5 {
            continue;
        }
        let pt = tokens(&stim);
        let pmh = minhash_sig(&pt, &a, &b);
        let psh = simhash(&pt);
        s_eng.push((engine.recognize(&stim)?.familiarity, true));
        s_mh.push((
            mh.iter().map(|e| minhash_sim(&pmh, e)).fold(0.0, f64::max),
            true,
        ));
        s_sh.push((
            sh.iter().map(|&e| simhash_sim(psh, e)).fold(0.0, f64::max),
            true,
        ));
        s_bm.push((bm25.max_score(&pt), true));
        pos_n += 1;
    }

    // ── Negatives: held-out + label-noise mask ──
    let mut noisy = Vec::new();
    let mut label_noise = 0usize;
    for (_, content) in &held_out {
        let is_noise = max_jaccard(&token_set(content), &enrolled_sets) >= LABEL_NOISE_JACCARD;
        if is_noise {
            label_noise += 1;
        }
        noisy.push(is_noise);
        let nt = tokens(content);
        let nmh = minhash_sig(&nt, &a, &b);
        let nsh = simhash(&nt);
        s_eng.push((engine.recognize(content)?.familiarity, false));
        s_mh.push((
            mh.iter().map(|e| minhash_sim(&nmh, e)).fold(0.0, f64::max),
            false,
        ));
        s_sh.push((
            sh.iter().map(|&e| simhash_sim(nsh, e)).fold(0.0, f64::max),
            false,
        ));
        s_bm.push((bm25.max_score(&nt), false));
    }
    let neg_n = s_eng.len() - pos_n;

    println!("== recognition: peak-pair engine vs classical near-dup baselines ==");
    println!("positive type: {mode}");
    println!("positives: {pos_n}   negatives (held-out): {neg_n}   clean negatives: {} (label-noise {label_noise})", neg_n - label_noise);
    println!("method                     raw-AUC   clean-AUC   cost/notes");
    let row = |name: &str, s: &[(f64, bool)], note: &str| {
        println!(
            "{name:24}   {:.4}    {:.4}     {note}",
            roc_auc(s),
            clean_auc(s, &noisy, pos_n)
        );
    };
    row("peak-pair engine", &s_eng, "model-free, auditable, ~1.5ms");
    row("MinHash (128, Jaccard)", &s_mh, "model-free, ~O(k) compare");
    row("SimHash (64-bit)", &s_sh, "model-free, 1 popcount compare");
    row(
        "BM25 (inverted index)",
        &s_bm,
        "model-free, lexical retrieval",
    );
    println!("(BGE-small-en-v1.5 max-cosine baseline: clean-AUC 0.8658 @ ~495ms — see embedding_baseline)");
    Ok(())
}
