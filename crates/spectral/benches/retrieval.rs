//! Spectral retrieval benchmarks.
//!
//! Run with: cargo bench --bench retrieval -p spectral
//!
//! See benches/METHODOLOGY.md for how to interpret results.

use criterion::{criterion_group, criterion_main, Criterion};
use spectral::{Brain, Visibility};
use std::collections::HashMap;
use std::time::Duration;

// ── Deterministic corpus generation ─────────────────────────────────

/// Fixed seed for reproducible corpus generation.
const SEED: u64 = 0x5BEC_78A1;

const WINGS: &[&str] = &[
    "engineering",
    "product",
    "infrastructure",
    "research",
    "operations",
];

const HALLS: &[&str] = &["fact", "discovery", "preference", "advice"];

/// Wing-specific keywords that the classifier will match.
const WING_KEYWORDS: &[(&str, &[&str])] = &[
    (
        "engineering",
        &["code", "build", "deploy", "refactor", "test"],
    ),
    (
        "product",
        &["feature", "user", "roadmap", "launch", "design"],
    ),
    (
        "infrastructure",
        &["server", "database", "monitoring", "scaling", "cluster"],
    ),
    (
        "research",
        &["experiment", "hypothesis", "paper", "model", "data"],
    ),
    (
        "operations",
        &["incident", "runbook", "on-call", "alert", "SLA"],
    ),
];

/// Hall-specific keywords that the classifier will match.
const HALL_KEYWORDS: &[(&str, &[&str])] = &[
    ("fact", &["decided", "chose", "agreed", "locked in"]),
    ("discovery", &["learned", "discovered", "found that"]),
    ("preference", &["prefers", "likes", "favourite"]),
    ("advice", &["should", "recommend", "suggest"]),
];

/// Simple LCG PRNG for deterministic generation without external deps.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn usize(&mut self, max: usize) -> usize {
        (self.next_u64() as usize) % max
    }
}

struct CorpusEntry {
    key: String,
    content: String,
    wing: String,
}

fn generate_corpus(n: usize) -> Vec<CorpusEntry> {
    let mut rng = Rng::new(SEED);
    let mut entries = Vec::with_capacity(n);

    for i in 0..n {
        let wing_idx = rng.usize(WINGS.len());
        let hall_idx = rng.usize(HALLS.len());
        let wing = WINGS[wing_idx];
        let hall = HALLS[hall_idx];

        let wing_kw = WING_KEYWORDS[wing_idx].1;
        let hall_kw = HALL_KEYWORDS[hall_idx].1;

        let w1 = wing_kw[rng.usize(wing_kw.len())];
        let w2 = wing_kw[rng.usize(wing_kw.len())];
        let h1 = hall_kw[rng.usize(hall_kw.len())];

        let content = format!("Memory {i}: {h1} the {wing} team {w1} approach works well for {w2}");

        entries.push(CorpusEntry {
            key: format!("{wing}-{hall}-{i}"),
            content,
            wing: wing.to_string(),
        });
    }

    entries
}

/// Populate a brain with N memories. Returns the brain.
fn populated_brain(n: usize) -> (tempfile::TempDir, Brain) {
    let tmp = tempfile::tempdir().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    let corpus = generate_corpus(n);

    for entry in &corpus {
        brain
            .remember(&entry.key, &entry.content, Visibility::Private)
            .unwrap();
    }

    (tmp, brain)
}

// ── Suite A: Ingest throughput ──────────────────────────────────────

fn bench_ingest(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest");
    group.measurement_time(Duration::from_secs(10));

    // Single remember against an empty brain
    group.bench_function("single_empty_brain", |b| {
        let tmp = tempfile::tempdir().unwrap();
        let brain = Brain::open(tmp.path()).unwrap();
        let mut i = 0u64;
        b.iter(|| {
            let key = format!("bench-key-{i}");
            brain
                .remember(&key, "Decided to use Clerk for auth", Visibility::Private)
                .unwrap();
            i += 1;
        });
    });

    // Batch of 100 into a fresh brain
    group.bench_function("batch_100", |b| {
        b.iter_with_setup(
            || {
                let tmp = tempfile::tempdir().unwrap();
                let brain = Brain::open(tmp.path()).unwrap();
                (tmp, brain)
            },
            |(_tmp, brain)| {
                let corpus = generate_corpus(100);
                for entry in &corpus {
                    brain
                        .remember(&entry.key, &entry.content, Visibility::Private)
                        .unwrap();
                }
            },
        );
    });

    // 10 remembers into a brain with 1000 existing memories
    // (tests fingerprint pairing cost with many peers)
    group.bench_function("into_populated_1000", |b| {
        let (_tmp, brain) = populated_brain(1000);
        let mut i = 0u64;
        b.iter(|| {
            let key = format!("new-memory-{i}");
            let content =
                format!("Memory {i}: decided the infrastructure scaling approach works for deploy");
            brain.remember(&key, &content, Visibility::Private).unwrap();
            i += 1;
        });
    });

    group.finish();
}

// ── Suite B: Recall latency ─────────────────────────────────────────

fn bench_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("recall");
    group.measurement_time(Duration::from_secs(10));

    // Small brain: 100 memories
    {
        let (_tmp, brain) = populated_brain(100);
        group.bench_function("small_100", |b| {
            b.iter(|| {
                brain
                    .recall(
                        "infrastructure scaling server database",
                        Visibility::Private,
                    )
                    .unwrap();
            });
        });
    }

    // Medium brain: 1000 memories
    {
        let (_tmp, brain) = populated_brain(1000);
        group.bench_function("medium_1000", |b| {
            b.iter(|| {
                brain
                    .recall(
                        "infrastructure scaling server database",
                        Visibility::Private,
                    )
                    .unwrap();
            });
        });
    }

    // No-match query — exercises FTS fallback
    {
        let (_tmp, brain) = populated_brain(1000);
        group.bench_function("no_match_1000", |b| {
            b.iter(|| {
                brain
                    .recall(
                        "quantum entanglement topological manifold",
                        Visibility::Private,
                    )
                    .unwrap();
            });
        });
    }

    group.finish();
}

// ── Suite C: Spectral vs TF-IDF vector baseline ─────────────────────

/// Simple TF-IDF vectorizer for the comparison baseline.
/// This is a bag-of-words approach with cosine similarity — the simplest
/// meaningful "vector search" baseline.
struct TfIdfIndex {
    doc_vecs: Vec<Vec<f64>>,
    #[allow(dead_code)]
    vocab: Vec<String>,
    vocab_idx: HashMap<String, usize>,
    idf: Vec<f64>,
}

impl TfIdfIndex {
    fn build(docs: &[String]) -> Self {
        // Build vocabulary
        let mut vocab_idx: HashMap<String, usize> = HashMap::new();
        for doc in docs {
            for word in tokenize(doc) {
                let len = vocab_idx.len();
                vocab_idx.entry(word).or_insert(len);
            }
        }

        let vocab: Vec<String> = {
            let mut v = vec![String::new(); vocab_idx.len()];
            for (word, &idx) in &vocab_idx {
                v[idx] = word.clone();
            }
            v
        };

        // Compute IDF
        let n = docs.len() as f64;
        let mut doc_freq = vec![0usize; vocab.len()];
        for doc in docs {
            let words: std::collections::HashSet<_> = tokenize(doc).collect();
            for word in words {
                if let Some(&idx) = vocab_idx.get(&word) {
                    doc_freq[idx] += 1;
                }
            }
        }
        let idf: Vec<f64> = doc_freq
            .iter()
            .map(|&df| {
                if df > 0 {
                    (n / df as f64).ln() + 1.0
                } else {
                    1.0
                }
            })
            .collect();

        // Compute TF-IDF vectors
        let doc_vecs: Vec<Vec<f64>> = docs
            .iter()
            .map(|doc| {
                let mut vec = vec![0.0; vocab.len()];
                let words: Vec<_> = tokenize(doc).collect();
                let len = words.len() as f64;
                for word in &words {
                    if let Some(&idx) = vocab_idx.get(word.as_str()) {
                        vec[idx] += 1.0 / len; // TF
                    }
                }
                for (i, v) in vec.iter_mut().enumerate() {
                    *v *= idf[i]; // TF * IDF
                }
                vec
            })
            .collect();

        Self {
            doc_vecs,
            vocab,
            vocab_idx,
            idf,
        }
    }

    fn query(&self, text: &str, k: usize) -> Vec<(usize, f64)> {
        // Build query vector
        let mut q_vec = vec![0.0; self.vocab.len()];
        let words: Vec<_> = tokenize(text).collect();
        let len = words.len().max(1) as f64;
        for word in &words {
            if let Some(&idx) = self.vocab_idx.get(word.as_str()) {
                q_vec[idx] += 1.0 / len;
            }
        }
        for (i, v) in q_vec.iter_mut().enumerate() {
            *v *= self.idf[i];
        }

        // Cosine similarity against all docs
        let q_norm = q_vec.iter().map(|x| x * x).sum::<f64>().sqrt();
        if q_norm < 1e-10 {
            return Vec::new();
        }

        let mut scores: Vec<(usize, f64)> = self
            .doc_vecs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let dot: f64 = d.iter().zip(q_vec.iter()).map(|(a, b)| a * b).sum();
                let d_norm = d.iter().map(|x| x * x).sum::<f64>().sqrt();
                let sim = if d_norm > 1e-10 {
                    dot / (q_norm * d_norm)
                } else {
                    0.0
                };
                (i, sim)
            })
            .collect();

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scores.truncate(k);
        scores
    }
}

fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_lowercase())
}

/// Test queries with known relevant wings (ground truth).
struct TestQuery {
    text: &'static str,
    relevant_wings: &'static [&'static str],
    is_multi_hop: bool,
}

const TEST_QUERIES: &[TestQuery] = &[
    TestQuery {
        text: "infrastructure scaling server database cluster",
        relevant_wings: &["infrastructure"],
        is_multi_hop: false,
    },
    TestQuery {
        text: "what did the engineering team decide about deploy",
        relevant_wings: &["engineering"],
        is_multi_hop: false,
    },
    TestQuery {
        text: "research experiment hypothesis data model",
        relevant_wings: &["research"],
        is_multi_hop: false,
    },
    // Multi-hop: queries that span concepts across wings
    TestQuery {
        text: "how does infrastructure scaling affect the deploy build",
        relevant_wings: &["infrastructure", "engineering"],
        is_multi_hop: true,
    },
    TestQuery {
        text: "operations incident monitoring and infrastructure cluster",
        relevant_wings: &["operations", "infrastructure"],
        is_multi_hop: true,
    },
];

fn bench_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(20);

    let n = 1000;
    let corpus = generate_corpus(n);

    // Build Spectral brain
    let tmp = tempfile::tempdir().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    for entry in &corpus {
        brain
            .remember(&entry.key, &entry.content, Visibility::Private)
            .unwrap();
    }

    // Build TF-IDF index (same corpus)
    let texts: Vec<String> = corpus.iter().map(|e| e.content.clone()).collect();
    let tfidf = TfIdfIndex::build(&texts);

    // Benchmark Spectral recall latency
    group.bench_function("spectral_recall", |b| {
        b.iter(|| {
            for q in TEST_QUERIES {
                brain.recall(q.text, Visibility::Private).unwrap();
            }
        });
    });

    // Benchmark TF-IDF recall latency
    group.bench_function("tfidf_recall", |b| {
        b.iter(|| {
            for q in TEST_QUERIES {
                tfidf.query(q.text, 5);
            }
        });
    });

    group.finish();

    // Quality comparison (print, not timed)
    println!("\n=== Retrieval Quality Comparison (1000 memories, top-5) ===");
    println!(
        "{:<55} {:>12} {:>12}",
        "Query", "Spectral P@5", "TF-IDF P@5"
    );
    println!("{:-<55} {:->12} {:->12}", "", "", "");

    let mut spectral_total_precision = 0.0;
    let mut tfidf_total_precision = 0.0;
    let mut spectral_multi_precision = 0.0;
    let mut tfidf_multi_precision = 0.0;
    let mut multi_count = 0;

    for q in TEST_QUERIES {
        // Spectral precision
        let s_result = brain.recall(q.text, Visibility::Private).unwrap();
        let s_hits: Vec<&str> = s_result
            .memory_hits
            .iter()
            .take(5)
            .filter_map(|h| h.wing.as_deref())
            .collect();
        let s_relevant = s_hits
            .iter()
            .filter(|w| q.relevant_wings.contains(w))
            .count();
        let s_precision = if s_hits.is_empty() {
            0.0
        } else {
            s_relevant as f64 / s_hits.len() as f64
        };

        // TF-IDF precision
        let t_results = tfidf.query(q.text, 5);
        let t_relevant = t_results
            .iter()
            .filter(|(idx, _)| q.relevant_wings.contains(&corpus[*idx].wing.as_str()))
            .count();
        let t_precision = if t_results.is_empty() {
            0.0
        } else {
            t_relevant as f64 / t_results.len() as f64
        };

        let label = if q.text.len() > 50 {
            format!("{}...", &q.text[..47])
        } else {
            q.text.to_string()
        };
        println!("{:<55} {:>11.2} {:>11.2}", label, s_precision, t_precision);

        spectral_total_precision += s_precision;
        tfidf_total_precision += t_precision;

        if q.is_multi_hop {
            spectral_multi_precision += s_precision;
            tfidf_multi_precision += t_precision;
            multi_count += 1;
        }
    }

    let n_queries = TEST_QUERIES.len() as f64;
    println!("{:-<55} {:->12} {:->12}", "", "", "");
    println!(
        "{:<55} {:>11.2} {:>11.2}",
        "Mean Precision@5 (all)",
        spectral_total_precision / n_queries,
        tfidf_total_precision / n_queries,
    );
    if multi_count > 0 {
        println!(
            "{:<55} {:>11.2} {:>11.2}",
            "Mean Precision@5 (multi-hop only)",
            spectral_multi_precision / multi_count as f64,
            tfidf_multi_precision / multi_count as f64,
        );
    }
}

criterion_group!(benches, bench_ingest, bench_recall, bench_comparison,);
criterion_main!(benches);
