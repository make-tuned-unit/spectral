//! Neural embedding baseline for recognition — the honest comparison point.
//!
//! Runs the EXACT protocol as `bin/replay.rs` (same 90/10 split, same ~30%
//! token dropout for positives, same held-out negatives, same Jaccard
//! label-noise mask, same rank-statistic AUC) but replaces the deterministic
//! peak-pair familiarity scalar with **max cosine similarity to any enrolled
//! embedding** (BGE-small-en-v1.5 via fastembed). The only variable is the
//! score, so the AUC gap is attributable to the method, not the harness.
//!
//! "Have I seen this before?" under embeddings = "is this near a memory I hold
//! in vector space?". This is what a standard embedding retrieval would do; the
//! recognition engine must at least match it while staying model-free and
//! auditable.
//!
//! Usage: cargo run --release --example embedding_baseline -- \
//!          --db ~/.permagent/brain/memory.db [--limit N]

use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use spectral_recognition::eval::{
    degrade, max_jaccard, roc_auc, split_9010, token_set, LABEL_NOISE_JACCARD,
};

/// L2-normalize in place so a dot product equals cosine similarity.
fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-10 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Max dot product (cosine, both sides normalized) of `probe` against every
/// enrolled vector.
fn max_cosine(probe: &[f32], enrolled: &[Vec<f32>]) -> f64 {
    enrolled
        .iter()
        .map(|e| e.iter().zip(probe).map(|(a, b)| a * b).sum::<f32>())
        .fold(f32::MIN, f32::max) as f64
}

fn embed_all(model: &mut TextEmbedding, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
    let mut out = model.embed(texts, Some(64))?;
    for v in out.iter_mut() {
        normalize(v);
    }
    Ok(out)
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

    eprintln!("initializing fastembed (BAAI/bge-small-en-v1.5)…");
    let mut model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGESmallENV15))
        .context("fastembed init (first run downloads the model)")?;

    // Enroll: embed every known memory.
    let t = std::time::Instant::now();
    let enrolled: Vec<Vec<f32>> =
        embed_all(&mut model, known.iter().map(|(_, c)| c.clone()).collect())?;
    eprintln!(
        "enroll: {:.1}ms total, {:.3}ms/memory",
        t.elapsed().as_secs_f64() * 1e3,
        t.elapsed().as_secs_f64() * 1e3 / known.len().max(1) as f64
    );

    // Positives: degraded enrolled memories (same dropout + min-token gate).
    let pos_texts: Vec<String> = known
        .iter()
        .map(|(id, c)| degrade(c, id, 30))
        .filter(|s| s.split_whitespace().count() >= 5)
        .collect();
    let t = std::time::Instant::now();
    let pos_vecs = embed_all(&mut model, pos_texts)?;
    let pos_n = pos_vecs.len();
    let query_ms = t.elapsed().as_secs_f64() * 1e3 / pos_n.max(1) as f64;

    let mut scores: Vec<(f64, bool)> = Vec::new();
    for v in &pos_vecs {
        scores.push((max_cosine(v, &enrolled), true));
    }

    // Negatives: held-out memories, plus the same Jaccard label-noise mask.
    let enrolled_sets: Vec<_> = known.iter().map(|(_, c)| token_set(c)).collect();
    let neg_vecs = embed_all(
        &mut model,
        held_out.iter().map(|(_, c)| c.clone()).collect(),
    )?;
    let mut noisy: Vec<bool> = Vec::new();
    let mut label_noise = 0usize;
    for ((_, content), v) in held_out.iter().zip(&neg_vecs) {
        let is_noise = max_jaccard(&token_set(content), &enrolled_sets) >= LABEL_NOISE_JACCARD;
        if is_noise {
            label_noise += 1;
        }
        noisy.push(is_noise);
        scores.push((max_cosine(v, &enrolled), false));
    }
    let neg_n = scores.len() - pos_n;

    let auc = roc_auc(&scores);
    let clean: Vec<(f64, bool)> = scores
        .iter()
        .enumerate()
        .filter(|(i, s)| s.1 || !noisy[*i - pos_n])
        .map(|(_, s)| *s)
        .collect();
    let clean_neg = clean.iter().filter(|s| !s.1).count();
    let auc_clean = roc_auc(&clean);

    println!("== embedding baseline (BGE-small-en-v1.5, max cosine) ==");
    println!("enrolled:              {}", known.len());
    println!("positives (degraded):  {pos_n}");
    println!("negatives (held-out):  {neg_n}");
    println!("AUC(cosine):           {auc:.4}");
    println!(
        "label-noise negatives: {label_noise} / {neg_n} (Jaccard >= {LABEL_NOISE_JACCARD} vs enrolled)"
    );
    println!("AUC(clean negatives):  {auc_clean:.4}  ({clean_neg} clean negatives)");
    println!("query latency:         {query_ms:.3} ms/query (embed only, batched)");
    Ok(())
}
