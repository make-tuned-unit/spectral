//! Permagent-realistic benchmark — Phase 0: T1 (ingest cost/throughput) and
//! T5 (determinism), Spectral vs MinHash+BM25, at $0.
//! See docs/internal/PERMAGENT_BENCHMARK_SPEC.md.
//!
//! What this measures, all on the REAL brain corpus (~1738 memories):
//!   T1  ingest wall-clock + events/sec + storage bytes/event, for
//!       (a) Spectral's real ambient-ingest path (classify+score+FTS+fingerprint)
//!       (b) a MinHash+BM25 classical index — Spectral's deterministic rival.
//!       Both are $0/event (no API). The embed-stack cost (pgvector/Mem0/Zep)
//!       is computed ANALYTICALLY from the corpus token count — no spend.
//!   T5  determinism — run each query twice, fraction byte-identical ranking.
//!
//! Auditability (the other half of T5) is a written rubric in the results doc,
//! not a number this bin emits.
//!
//! Usage: phase0 --brain ~/.permagent/brain/memory.db [--limit N] [--queries N]

use anyhow::{Context, Result};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use spectral_ingest::ingest::{ingest_with, IngestConfig, IngestOpts};
use spectral_ingest::sqlite_store::SqliteStore;
use spectral_ingest::MemoryStore;

/// A 2025 top embedder's input price, for the analytical embed-stack cost.
/// text-embedding-3-large: $0.13 / 1M input tokens (documented list price).
const EMBED_USD_PER_1M_TOKENS: f64 = 0.13;
/// Real Permagent ingest rate, from the spec: ~1738 memories over ~48 days.
const CORPUS_SPAN_DAYS: f64 = 48.0;

const MINHASH_K: usize = 128;

// ── record loaded from the real brain ──────────────────────────────
struct Doc {
    id: String,
    key: String,
    content: String,
    category: String,
    visibility: String,
}

fn load_corpus(brain: &PathBuf, limit: Option<usize>) -> Result<Vec<Doc>> {
    let conn = Connection::open(brain).with_context(|| format!("open {brain:?}"))?;
    let mut stmt = conn.prepare(
        "SELECT id, key, content, category, COALESCE(visibility,'private')
         FROM memories ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Doc {
            id: r.get(0)?,
            key: r.get(1)?,
            content: r.get(2)?,
            category: r.get(3)?,
            visibility: r.get(4)?,
        })
    })?;
    let mut docs: Vec<Doc> = rows.collect::<Result<_, _>>()?;
    if let Some(n) = limit {
        docs.truncate(n);
    }
    Ok(docs)
}

// ── tokenization shared by BM25 and query building ─────────────────
fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_lowercase())
        .collect()
}

fn hash64(s: &str) -> u64 {
    let d = Sha256::digest(s.as_bytes());
    u64::from_be_bytes(d[..8].try_into().unwrap())
}
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// A minimal MinHash + BM25 index — the classical deterministic rival.
struct Classical {
    a: Vec<u64>,
    b: Vec<u64>,
    sigs: Vec<[u64; MINHASH_K]>,           // MinHash signature per doc
    postings: HashMap<String, Vec<(usize, u32)>>, // term -> (doc, tf)
    doc_len: Vec<u32>,
    avg_len: f64,
    n: usize,
}

impl Classical {
    fn build(docs: &[Doc]) -> Self {
        let a: Vec<u64> = (0..MINHASH_K).map(|i| splitmix64(i as u64 * 2 + 1) | 1).collect();
        let b: Vec<u64> = (0..MINHASH_K).map(|i| splitmix64(i as u64 * 2 + 2)).collect();
        let mut sigs = Vec::with_capacity(docs.len());
        let mut postings: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
        let mut doc_len = Vec::with_capacity(docs.len());
        let mut total_len: u64 = 0;

        for (d, doc) in docs.iter().enumerate() {
            let toks = tokens(&doc.content);
            doc_len.push(toks.len() as u32);
            total_len += toks.len() as u64;

            // BM25 term frequencies
            let mut tf: HashMap<&str, u32> = HashMap::new();
            for t in &toks {
                *tf.entry(t.as_str()).or_insert(0) += 1;
            }
            for (term, f) in tf {
                postings.entry(term.to_string()).or_default().push((d, f));
            }

            // MinHash signature over the distinct token set
            let mut sig = [u64::MAX; MINHASH_K];
            let distinct: HashSet<u64> = toks.iter().map(|t| hash64(t)).collect();
            for base in distinct {
                for i in 0..MINHASH_K {
                    let h = base.wrapping_mul(a[i]).wrapping_add(b[i]);
                    if h < sig[i] {
                        sig[i] = h;
                    }
                }
            }
            sigs.push(sig);
        }
        let n = docs.len();
        Classical {
            a,
            b,
            sigs,
            postings,
            avg_len: if n > 0 { total_len as f64 / n as f64 } else { 0.0 },
            doc_len,
            n,
        }
    }

    /// BM25 top-k over query terms — deterministic ranking (doc_id, score).
    fn bm25(&self, query_terms: &[String], k: usize) -> Vec<(usize, f64)> {
        const K1: f64 = 1.5;
        const B: f64 = 0.75;
        let mut scores: HashMap<usize, f64> = HashMap::new();
        for term in query_terms {
            if let Some(list) = self.postings.get(term) {
                let idf = ((self.n as f64 - list.len() as f64 + 0.5) / (list.len() as f64 + 0.5)
                    + 1.0)
                    .ln();
                for &(doc, f) in list {
                    let f = f as f64;
                    let dl = self.doc_len[doc] as f64;
                    let denom = f + K1 * (1.0 - B + B * dl / self.avg_len);
                    *scores.entry(doc).or_insert(0.0) += idf * (f * (K1 + 1.0)) / denom;
                }
            }
        }
        let mut v: Vec<(usize, f64)> = scores.into_iter().collect();
        // deterministic tie-break: score desc, then doc id asc
        v.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap().then(x.0.cmp(&y.0)));
        v.truncate(k);
        v
    }

    /// Approx in-RAM footprint (bytes): signatures + postings + lengths.
    fn ram_bytes(&self) -> usize {
        let sig = self.sigs.len() * MINHASH_K * 8;
        let post: usize = self
            .postings
            .iter()
            .map(|(t, l)| t.len() + 24 + l.len() * 12)
            .sum();
        let lens = self.doc_len.len() * 4;
        let perm = (self.a.len() + self.b.len()) * 8;
        sig + post + lens + perm
    }
}

/// Build ~`n` deterministic queries from the corpus: first 6 tokens of every
/// stride-th memory. Relevance is irrelevant — determinism is input-invariant.
fn build_queries(docs: &[Doc], n: usize) -> Vec<Vec<String>> {
    if docs.is_empty() || n == 0 {
        return vec![];
    }
    let stride = (docs.len() / n).max(1);
    docs.iter()
        .step_by(stride)
        .filter_map(|d| {
            let q: Vec<String> = tokens(&d.content).into_iter().take(6).collect();
            (!q.is_empty()).then_some(q)
        })
        .collect()
}

fn dir_bytes(path: &PathBuf) -> u64 {
    let mut total = 0;
    for suffix in ["", "-wal", "-shm"] {
        let p = if suffix.is_empty() {
            path.clone()
        } else {
            PathBuf::from(format!("{}{}", path.display(), suffix))
        };
        if let Ok(m) = std::fs::metadata(&p) {
            total += m.len();
        }
    }
    total
}

#[derive(clap::Parser)]
#[command(about = "Permagent benchmark Phase 0 — ingest cost + determinism")]
struct Cli {
    #[arg(long, default_value = "~/.permagent/brain/memory.db")]
    brain: String,
    /// Cap corpus size (default: all).
    #[arg(long)]
    limit: Option<usize>,
    /// Number of determinism probe queries.
    #[arg(long, default_value_t = 100)]
    queries: usize,
    /// Write JSON report here.
    #[arg(long)]
    out: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = <Cli as clap::Parser>::parse();
    let brain = PathBuf::from(shellexpand(&cli.brain));
    let docs = load_corpus(&brain, cli.limit)?;
    anyhow::ensure!(!docs.is_empty(), "no memories loaded from {brain:?}");
    let n = docs.len();
    eprintln!("loaded {n} memories from {}", brain.display());

    // corpus token stats (for the analytical embed cost)
    let total_chars: usize = docs.iter().map(|d| d.content.len()).sum();
    let total_words: usize = docs.iter().map(|d| d.content.split_whitespace().count()).sum();
    let est_tokens = (total_chars as f64 / 4.0).round() as u64; // OpenAI ~4 chars/token

    // ── T1a: Spectral ambient-ingest path ──────────────────────────
    let tmp = std::env::temp_dir().join(format!("phase0_spectral_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let store = SqliteStore::open(&tmp)?;
    let cfg = IngestConfig::default();
    let t = Instant::now();
    for d in &docs {
        ingest_with(
            &d.id,
            &d.key,
            &d.content,
            &d.category,
            0.0,
            &d.visibility,
            &cfg,
            &store,
            IngestOpts::default(),
        )
        .await
        .with_context(|| format!("ingest {}", d.key))?;
    }
    let spectral_secs = t.elapsed().as_secs_f64();
    // total on-disk footprint = main db + WAL + shm (summed in dir_bytes)
    let spectral_bytes = dir_bytes(&tmp);

    // ── T1b: classical MinHash+BM25 index ──────────────────────────
    let t = Instant::now();
    let classical = Classical::build(&docs);
    let classical_secs = t.elapsed().as_secs_f64();
    let classical_bytes = classical.ram_bytes() as u64;

    // ── T5: determinism ────────────────────────────────────────────
    let queries = build_queries(&docs, cli.queries);
    let mut spectral_stable = 0usize;
    for q in &queries {
        let r1 = store.fts_search(q, 10).await?;
        let r2 = store.fts_search(q, 10).await?;
        let k1: Vec<(&str, u64)> = r1.iter().map(|h| (h.id.as_str(), h.signal_score.to_bits())).collect();
        let k2: Vec<(&str, u64)> = r2.iter().map(|h| (h.id.as_str(), h.signal_score.to_bits())).collect();
        if k1 == k2 {
            spectral_stable += 1;
        }
    }
    let mut classical_stable = 0usize;
    for q in &queries {
        let r1 = classical.bm25(q, 10);
        let r2 = classical.bm25(q, 10);
        let k1: Vec<(usize, u64)> = r1.iter().map(|(d, s)| (*d, s.to_bits())).collect();
        let k2: Vec<(usize, u64)> = r2.iter().map(|(d, s)| (*d, s.to_bits())).collect();
        if k1 == k2 {
            classical_stable += 1;
        }
    }
    let nq = queries.len().max(1);

    // ── analytical embed-stack cost (no spend) ─────────────────────
    let ingest_cost_usd = est_tokens as f64 / 1e6 * EMBED_USD_PER_1M_TOKENS;
    let events_per_day = n as f64 / CORPUS_SPAN_DAYS;
    let events_per_month = events_per_day * 30.0;
    let tokens_per_event = est_tokens as f64 / n as f64;
    let monthly_embed_usd = events_per_month * tokens_per_event / 1e6 * EMBED_USD_PER_1M_TOKENS;

    let report = serde_json::json!({
        "corpus": {
            "memories": n,
            "total_chars": total_chars,
            "total_words": total_words,
            "est_tokens": est_tokens,
            "span_days": CORPUS_SPAN_DAYS,
            "events_per_day": events_per_day,
        },
        "T1_ingest": {
            "spectral": {
                "secs": spectral_secs,
                "events_per_sec": n as f64 / spectral_secs,
                "storage_bytes": spectral_bytes,
                "bytes_per_event": spectral_bytes as f64 / n as f64,
                "api_cost_usd": 0.0,
                "on_device": true,
            },
            "minhash_bm25": {
                "secs": classical_secs,
                "events_per_sec": n as f64 / classical_secs,
                "ram_bytes": classical_bytes,
                "bytes_per_event": classical_bytes as f64 / n as f64,
                "api_cost_usd": 0.0,
                "on_device": true,
            },
            "embed_stack_analytical": {
                "embedder": "text-embedding-3-large",
                "usd_per_1m_tokens": EMBED_USD_PER_1M_TOKENS,
                "ingest_cost_usd_full_corpus": ingest_cost_usd,
                "usd_per_1k_events": tokens_per_event * 1000.0 / 1e6 * EMBED_USD_PER_1M_TOKENS,
                "projected_usd_per_month": monthly_embed_usd,
                "on_device": false,
            }
        },
        "T5_determinism": {
            "probe_queries": nq,
            "spectral_fts5_stable_frac": spectral_stable as f64 / nq as f64,
            "minhash_bm25_stable_frac": classical_stable as f64 / nq as f64,
            "embedding_ann_note": "not measured here; HNSW/IVF rankings drift on index rebuild + ef/probe params — <1.0 by construction",
        }
    });

    let out = serde_json::to_string_pretty(&report)?;
    println!("{out}");
    if let Some(path) = cli.out {
        std::fs::write(&path, &out)?;
        eprintln!("wrote {path}");
    }
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(format!("{}-wal", tmp.display()));
    let _ = std::fs::remove_file(format!("{}-shm", tmp.display()));

    // human summary
    eprintln!("\n── Phase 0 summary ──────────────────────────────");
    eprintln!("corpus:      {n} memories, ~{est_tokens} tokens");
    eprintln!(
        "Spectral:    {:.0} ev/s, {:.0} B/ev, $0 API, on-device",
        n as f64 / spectral_secs,
        spectral_bytes as f64 / n as f64
    );
    eprintln!(
        "MinHash+BM25:{:.0} ev/s, {:.0} B/ev RAM, $0 API, on-device",
        n as f64 / classical_secs,
        classical_bytes as f64 / n as f64
    );
    eprintln!(
        "Embed stack: ${:.4} full ingest, ~${:.2}/mo @ {:.0} ev/day, NOT on-device",
        ingest_cost_usd, monthly_embed_usd, events_per_day
    );
    eprintln!(
        "Determinism: Spectral {:.1}%, BM25 {:.1}%  (embed-ANN <100% by construction)",
        100.0 * spectral_stable as f64 / nq as f64,
        100.0 * classical_stable as f64 / nq as f64
    );
    Ok(())
}

fn shellexpand(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    p.to_string()
}
