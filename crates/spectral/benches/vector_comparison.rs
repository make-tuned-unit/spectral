//! Neural vector comparison benchmark: Spectral vs fastembed (BGE-small-en-v1.5).
//!
//! Run with: cargo bench --bench vector_comparison -p spectral
//!
//! See benches/METHODOLOGY.md for interpretation.
//!
//! First run downloads the ONNX model (~130 MB) from HuggingFace and the
//! ONNX Runtime binary (~200 MB). Subsequent runs use the local cache.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use spectral::{Brain, Visibility};

// ── Deterministic RNG (shared with retrieval.rs) ─────────────────────

const SEED: u64 = 0x5BEC_78A1;

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

// ── Corpus generation ────────────────────────────────────────────────
//
// Uses vocabulary matching production wing/hall rules so TACT's
// fingerprint search actually fires. Each wing's vocabulary is
// isolated to prevent cross-wing mis-classification.

struct WingVocab {
    name: &'static str,
    triggers: &'static [&'static str],
    topics: &'static [&'static str],
    details: &'static [&'static str],
}

const WING_VOCABS: &[WingVocab] = &[
    WingVocab {
        name: "apollo",
        triggers: &[
            "apollo",
            "prediction-market",
            "weather",
            "prediction",
            "wager",
            "trade",
        ],
        topics: &[
            "accuracy on the latest predictions",
            "signal analysis from market data",
            "forecast integration pipeline",
            "portfolio optimization results",
            "backtesting on recent wager data",
        ],
        details: &[
            "The prediction-market platform showed strong signals",
            "Weather API response times improved significantly",
            "Wager sizing algorithm performed well",
            "Trade execution was within acceptable latency",
            "Prediction model v2 outperformed the baseline",
        ],
    },
    WingVocab {
        name: "acme",
        triggers: &["acme", "widget", "bob", "recipe", "cook", "feast"],
        topics: &[
            "testing the new recipe database",
            "meal planning feature updates",
            "ingredient sourcing automation",
            "feast preparation workflow",
            "menu design improvements",
        ],
        details: &[
            "Bob's feedback shaped the final widget interface",
            "Cook time estimation accuracy is at 94 percent",
            "Acme feast mode handles groups of twenty plus",
            "Recipe sharing between users works smoothly",
            "The widget app search got noticeably faster",
        ],
    },
    WingVocab {
        name: "infra",
        triggers: &["infrastructure", "ollama", "taskforge", "litellm", "gemma"],
        topics: &[
            "serving pipeline for the cluster",
            "deployment automation improvements",
            "GPU utilization metrics update",
            "API gateway configuration",
            "container orchestration changes",
        ],
        details: &[
            "Ollama cluster handled 2x peak load",
            "Litellm proxy routing is stable",
            "Taskforge task runner scaled to 50 concurrent jobs",
            "Gemma model weights updated to latest checkpoint",
            "Infrastructure monitoring dashboards were deployed",
        ],
    },
    WingVocab {
        name: "alice",
        triggers: &["alice", "coffee", "anniversary", "colour", "noah", "leo"],
        topics: &[
            "morning routine with the family",
            "planning for the upcoming event",
            "home office setup changes",
            "weekend activities with the kids",
            "personal goals for the quarter",
        ],
        details: &[
            "Alice found a new single-origin roast",
            "The anniversary celebration went really well",
            "Noah and Leo enjoyed the outing",
            "Coffee brewing method changed to pour-over",
            "The colour scheme for the office was finalized",
        ],
    },
    WingVocab {
        name: "polaris",
        triggers: &["polaris", "plr", "plogging", "summit", "marathon"],
        topics: &[
            "route planning for the next event",
            "volunteer coordination update",
            "cleanup metrics from the last session",
            "event scheduling for the season",
            "environmental impact assessment",
        ],
        details: &[
            "PLR tracking showed 15km covered",
            "Plogging session collected 30kg of debris",
            "Summit event registration is at capacity",
            "Marathon route avoids construction zones",
            "Polaris visibility increased 40 percent this quarter",
        ],
    },
];

// Hall starters that trigger hall classification without triggering any wing rule.
// "favourite/favorite" is avoided outside alice (it triggers the alice wing rule).
const FACT_STARTERS: &[&str] = &[
    "Decided to",
    "Chose the",
    "Agreed on the",
    "Will use the",
    "Locked in the",
];

const PREFERENCE_STARTERS: &[&str] = &["Prefers the", "Likes the", "Really likes the"];

const DISCOVERY_STARTERS: &[&str] = &[
    "Learned that",
    "Discovered the",
    "Found that the",
    "Realized the",
];

const ADVICE_STARTERS: &[&str] = &[
    "Should try the",
    "Recommend the",
    "Suggest using the",
    "Try using the",
];

const HALLS: &[&str] = &["fact", "preference", "discovery", "advice"];

struct CorpusEntry {
    key: String,
    content: String,
    wing: String,
    #[allow(dead_code)]
    hall: String,
}

fn hall_starter(hall_idx: usize, rng: &mut Rng) -> &'static str {
    match hall_idx {
        0 => FACT_STARTERS[rng.usize(FACT_STARTERS.len())],
        1 => PREFERENCE_STARTERS[rng.usize(PREFERENCE_STARTERS.len())],
        2 => DISCOVERY_STARTERS[rng.usize(DISCOVERY_STARTERS.len())],
        3 => ADVICE_STARTERS[rng.usize(ADVICE_STARTERS.len())],
        _ => unreachable!(),
    }
}

fn generate_corpus(n: usize) -> Vec<CorpusEntry> {
    let mut rng = Rng::new(SEED);
    let mut entries = Vec::with_capacity(n);

    for i in 0..n {
        let wing_idx = rng.usize(WING_VOCABS.len());
        let hall_idx = rng.usize(HALLS.len());
        let wing = &WING_VOCABS[wing_idx];
        let hall = HALLS[hall_idx];

        let starter = hall_starter(hall_idx, &mut rng);
        let trigger = wing.triggers[rng.usize(wing.triggers.len())];
        let topic = wing.topics[rng.usize(wing.topics.len())];
        let detail = wing.details[rng.usize(wing.details.len())];

        let content = format!("{starter} {trigger} {topic}. {detail}.");

        entries.push(CorpusEntry {
            key: format!("{}-{}-{i}", wing.name, hall),
            content,
            wing: wing.name.to_string(),
            hall: hall.to_string(),
        });
    }

    entries
}

// ── Query set ────────────────────────────────────────────────────────
//
// 20 queries across 4 categories (5 each). Generation rules documented
// in METHODOLOGY.md.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum QueryCategory {
    KeywordOverlap,  // exact corpus vocabulary (baseline for both)
    Paraphrase,      // different vocabulary, same meaning (vector-favored)
    MultiHopTopical, // triggers wing+hall, cross-hall fingerprints (Spectral-favored)
    VocabBridge,     // bridges concepts without trigger words (hard for both)
}

impl QueryCategory {
    fn label(self) -> &'static str {
        match self {
            Self::KeywordOverlap => "Keyword overlap",
            Self::Paraphrase => "Paraphrase",
            Self::MultiHopTopical => "Multi-hop topical",
            Self::VocabBridge => "Vocabulary bridge",
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Self::KeywordOverlap => "[KW]",
            Self::Paraphrase => "[PA]",
            Self::MultiHopTopical => "[MH]",
            Self::VocabBridge => "[VB]",
        }
    }
}

struct ComparisonQuery {
    text: &'static str,
    relevant_wings: &'static [&'static str],
    category: QueryCategory,
}

const COMPARISON_QUERIES: &[ComparisonQuery] = &[
    // ── Keyword overlap: uses exact corpus vocabulary. Both systems should
    //    perform well; measures baseline retrieval capability. ──
    ComparisonQuery {
        text: "apollo weather prediction wager trade",
        relevant_wings: &["apollo"],
        category: QueryCategory::KeywordOverlap,
    },
    ComparisonQuery {
        text: "acme recipe cook feast widget bob",
        relevant_wings: &["acme"],
        category: QueryCategory::KeywordOverlap,
    },
    ComparisonQuery {
        text: "infrastructure ollama litellm taskforge gemma",
        relevant_wings: &["infra"],
        category: QueryCategory::KeywordOverlap,
    },
    ComparisonQuery {
        text: "alice coffee anniversary colour noah leo",
        relevant_wings: &["alice"],
        category: QueryCategory::KeywordOverlap,
    },
    ComparisonQuery {
        text: "polaris plr marathon plogging summit",
        relevant_wings: &["polaris"],
        category: QueryCategory::KeywordOverlap,
    },
    // ── Paraphrase: rephrases the same concepts with different vocabulary.
    //    No wing trigger words. TACT falls to FTS; vector should win via
    //    semantic similarity. ──
    ComparisonQuery {
        text: "how accurate is the automated forecasting system for betting markets",
        relevant_wings: &["apollo"],
        category: QueryCategory::Paraphrase,
    },
    ComparisonQuery {
        text: "what culinary preparations are available in the meal platform",
        relevant_wings: &["acme"],
        category: QueryCategory::Paraphrase,
    },
    ComparisonQuery {
        text: "how is the neural network hosting cluster performing on GPU tasks",
        relevant_wings: &["infra"],
        category: QueryCategory::Paraphrase,
    },
    ComparisonQuery {
        text: "daily caffeine habits and family life updates from the dad",
        relevant_wings: &["alice"],
        category: QueryCategory::Paraphrase,
    },
    ComparisonQuery {
        text: "organized jogging conservation event results from the coastal area",
        relevant_wings: &["polaris"],
        category: QueryCategory::Paraphrase,
    },
    // ── Multi-hop topical: triggers wing+hall via TACT. Fingerprint search
    //    retrieves cross-hall memories within the detected wing — memories
    //    that share no query vocabulary but are fingerprint-linked. ──
    ComparisonQuery {
        text: "apollo decided which weather prediction model to deploy",
        relevant_wings: &["apollo"],
        category: QueryCategory::MultiHopTopical,
    },
    ComparisonQuery {
        text: "acme discovered a breakthrough recipe for the feast",
        relevant_wings: &["acme"],
        category: QueryCategory::MultiHopTopical,
    },
    ComparisonQuery {
        text: "infrastructure should recommend which ollama model to use",
        relevant_wings: &["infra"],
        category: QueryCategory::MultiHopTopical,
    },
    ComparisonQuery {
        text: "alice learned that her morning coffee routine needs noah leo time",
        relevant_wings: &["alice"],
        category: QueryCategory::MultiHopTopical,
    },
    ComparisonQuery {
        text: "polaris decided the marathon plogging route for summit",
        relevant_wings: &["polaris"],
        category: QueryCategory::MultiHopTopical,
    },
    // ── Vocabulary bridge: completely different vocabulary, bridging
    //    concepts across domains. No wing trigger words. Hard for both. ──
    ComparisonQuery {
        text: "automated system compute costs for the data pipeline",
        relevant_wings: &["infra"],
        category: QueryCategory::VocabBridge,
    },
    ComparisonQuery {
        text: "nutritional tracking analytics in community exercise events",
        relevant_wings: &["acme", "polaris"],
        category: QueryCategory::VocabBridge,
    },
    ComparisonQuery {
        text: "personal morning ritual with artisan beverages and family time",
        relevant_wings: &["alice"],
        category: QueryCategory::VocabBridge,
    },
    ComparisonQuery {
        text: "deployment efficiency metrics for real-time data services",
        relevant_wings: &["infra", "apollo"],
        category: QueryCategory::VocabBridge,
    },
    ComparisonQuery {
        text: "community event scheduling platform with shared meal coordination",
        relevant_wings: &["polaris", "acme"],
        category: QueryCategory::VocabBridge,
    },
];

// ── Neural vector index ──────────────────────────────────────────────

fn init_embedding_model(show_progress: bool) -> TextEmbedding {
    let mut opts = InitOptions::new(EmbeddingModel::BGESmallENV15);
    opts.show_download_progress = show_progress;
    TextEmbedding::try_new(opts).expect("failed to initialize BGE-small-en-v1.5 embedding model")
}

struct NeuralVectorIndex {
    model: RefCell<TextEmbedding>,
    doc_embeddings: Vec<Vec<f32>>,
}

impl NeuralVectorIndex {
    fn build(docs: &[String]) -> Self {
        let mut model = init_embedding_model(true);

        // Batch-encode all documents
        let doc_refs: Vec<&str> = docs.iter().map(String::as_str).collect();
        let doc_embeddings = model
            .embed(doc_refs, Some(64))
            .expect("failed to embed documents");

        Self {
            model: RefCell::new(model),
            doc_embeddings,
        }
    }

    /// Encode query + search (cold path: includes encoding latency).
    fn query_cold(&self, text: &str, k: usize) -> Vec<(usize, f32)> {
        let query_emb = self
            .model
            .borrow_mut()
            .embed(vec![text], None)
            .expect("query embedding failed");
        self.search(&query_emb[0], k)
    }

    /// Pre-encode a query for warm benchmarking.
    fn encode_query(&self, text: &str) -> Vec<f32> {
        self.model
            .borrow_mut()
            .embed(vec![text], None)
            .expect("query embedding failed")
            .into_iter()
            .next()
            .unwrap()
    }

    /// Search with a pre-encoded query (warm path: no encoding latency).
    fn query_warm(&self, pre_encoded: &[f32], k: usize) -> Vec<(usize, f32)> {
        self.search(pre_encoded, k)
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<(usize, f32)> {
        let mut scores: Vec<(usize, f32)> = self
            .doc_embeddings
            .iter()
            .enumerate()
            .map(|(i, doc)| (i, cosine_similarity(query, doc)))
            .collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores.truncate(k);
        scores
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < 1e-10 || norm_b < 1e-10 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

// ── Metrics ──────────────────────────────────────────────────────────

fn precision_at_k(retrieved_wings: &[Option<&str>], relevant_wings: &[&str]) -> f64 {
    if retrieved_wings.is_empty() {
        return 0.0;
    }
    let relevant_count = retrieved_wings
        .iter()
        .filter(|w| w.is_some_and(|w| relevant_wings.contains(&w)))
        .count();
    relevant_count as f64 / retrieved_wings.len() as f64
}

fn recall_at_k(
    retrieved_wings: &[Option<&str>],
    relevant_wings: &[&str],
    total_relevant: usize,
) -> f64 {
    if total_relevant == 0 {
        return 0.0;
    }
    let relevant_count = retrieved_wings
        .iter()
        .filter(|w| w.is_some_and(|w| relevant_wings.contains(&w)))
        .count();
    relevant_count as f64 / total_relevant as f64
}

fn f1(precision: f64, recall: f64) -> f64 {
    if precision + recall < 1e-10 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}

// ── Claim 1 + 2: Speed and accuracy ─────────────────────────────────

fn bench_speed_and_accuracy(c: &mut Criterion) {
    let n = 1000;
    let corpus = generate_corpus(n);

    // Wing counts for recall denominator
    let wing_counts: HashMap<&str, usize> = {
        let mut counts = HashMap::new();
        for entry in &corpus {
            *counts.entry(entry.wing.as_str()).or_insert(0) += 1;
        }
        counts
    };

    // Build Spectral brain
    let tmp = tempfile::tempdir().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    for entry in &corpus {
        brain
            .remember(&entry.key, &entry.content, Visibility::Private)
            .unwrap();
    }

    // Build neural vector index
    println!("\n  Initializing fastembed (BAAI/bge-small-en-v1.5)...");
    let texts: Vec<String> = corpus.iter().map(|e| e.content.clone()).collect();
    let vector_idx = NeuralVectorIndex::build(&texts);
    println!(
        "  Model loaded. {} documents, {} dimensions.",
        vector_idx.doc_embeddings.len(),
        vector_idx.doc_embeddings[0].len()
    );

    // Pre-encode all queries for warm comparison
    let pre_encoded: Vec<Vec<f32>> = COMPARISON_QUERIES
        .iter()
        .map(|q| vector_idx.encode_query(q.text))
        .collect();

    // ── Claim 1: Speed comparison (criterion-timed) ──
    let mut group = c.benchmark_group("vector_comparison_speed");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(20);

    group.bench_function("spectral_recall_20q", |b| {
        b.iter(|| {
            for q in COMPARISON_QUERIES {
                black_box(brain.recall(q.text, Visibility::Private).unwrap());
            }
        });
    });

    group.bench_function("vector_cold_20q", |b| {
        b.iter(|| {
            for q in COMPARISON_QUERIES {
                black_box(vector_idx.query_cold(q.text, 5));
            }
        });
    });

    group.bench_function("vector_warm_20q", |b| {
        b.iter(|| {
            for emb in &pre_encoded {
                black_box(vector_idx.query_warm(emb, 5));
            }
        });
    });

    group.finish();

    // ── Claim 2: Multi-hop accuracy (printed, not timed) ──
    println!("\n=== Neural Vector Comparison: Retrieval Quality ({n} memories, top-5) ===");
    println!("Legend: S=Spectral, V=Vector, P=Precision, R=Recall, F=F1\n");
    println!(
        "{:<4} {:<60} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5}",
        "Cat", "Query", "S-P@5", "S-R@5", "S-F1", "V-P@5", "V-R@5", "V-F1",
    );
    println!(
        "{:-<4} {:-<60} {:->5} {:->5} {:->5} {:->5} {:->5} {:->5}",
        "", "", "", "", "", "", "", ""
    );

    type CatVecs = (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>);
    let mut cat_metrics: HashMap<QueryCategory, CatVecs> = HashMap::new();

    for q in COMPARISON_QUERIES {
        let total_relevant: usize = q
            .relevant_wings
            .iter()
            .filter_map(|w| wing_counts.get(w))
            .sum();

        // Spectral
        let s_result = brain.recall(q.text, Visibility::Private).unwrap();
        let s_wings: Vec<Option<&str>> = s_result
            .memory_hits
            .iter()
            .take(5)
            .map(|h| h.wing.as_deref())
            .collect();
        let s_p = precision_at_k(&s_wings, q.relevant_wings);
        let s_r = recall_at_k(&s_wings, q.relevant_wings, total_relevant);
        let s_f = f1(s_p, s_r);

        // Vector
        let v_results = vector_idx.query_cold(q.text, 5);
        let v_wings: Vec<Option<&str>> = v_results
            .iter()
            .map(|(idx, _)| Some(corpus[*idx].wing.as_str()))
            .collect();
        let v_p = precision_at_k(&v_wings, q.relevant_wings);
        let v_r = recall_at_k(&v_wings, q.relevant_wings, total_relevant);
        let v_f = f1(v_p, v_r);

        let (sp, sr, vp, vr) = cat_metrics.entry(q.category).or_default();
        sp.push(s_p);
        sr.push(s_r);
        vp.push(v_p);
        vr.push(v_r);

        let label = if q.text.len() > 57 {
            format!("{}...", &q.text[..54])
        } else {
            q.text.to_string()
        };
        println!(
            "{:<4} {:<60} {:>5.2} {:>5.2} {:>5.3} {:>5.2} {:>5.2} {:>5.3}",
            q.category.tag(),
            label,
            s_p,
            s_r,
            s_f,
            v_p,
            v_r,
            v_f,
        );
    }

    // Category summaries
    println!(
        "\n{:<20} {:>12} {:>12} {:>12} {:>12}",
        "Category", "Spectral P@5", "Vector P@5", "Spectral F1", "Vector F1"
    );
    println!(
        "{:-<20} {:->12} {:->12} {:->12} {:->12}",
        "", "", "", "", ""
    );

    for cat in [
        QueryCategory::KeywordOverlap,
        QueryCategory::Paraphrase,
        QueryCategory::MultiHopTopical,
        QueryCategory::VocabBridge,
    ] {
        if let Some((sp, sr, vp, vr)) = cat_metrics.get(&cat) {
            let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
            let s_pm = mean(sp);
            let s_rm = mean(sr);
            let v_pm = mean(vp);
            let v_rm = mean(vr);
            println!(
                "{:<20} {:>11.2} {:>11.2} {:>11.3} {:>11.3}",
                cat.label(),
                s_pm,
                v_pm,
                f1(s_pm, s_rm),
                f1(v_pm, v_rm),
            );
        }
    }

    println!("\nNote: Recall@5 denominator is total memories in wing(s) (~200 per wing).");
    println!("      Maximum possible recall@5 = 5/200 = 0.025. Both systems are recall-bounded.");
    println!("      Precision@5 is the more meaningful comparison metric here.");
}

// ── Claim 3: Operational cost ────────────────────────────────────────

fn bench_operational(c: &mut Criterion) {
    // Quick criterion benchmark for cold-start comparison
    let mut group = c.benchmark_group("operational_cold_start");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function("spectral_brain_open", |b| {
        b.iter_with_setup(
            || tempfile::tempdir().unwrap(),
            |tmp| {
                black_box(Brain::open(tmp.path()).unwrap());
            },
        );
    });

    group.finish();

    // ── Printed operational measurements ──
    println!("\n=== Claim 3: Operational Cost Comparison ===\n");

    // Disk footprint
    println!("--- Disk footprint (1000 memories) ---");
    {
        let corpus = generate_corpus(1000);
        let tmp = tempfile::tempdir().unwrap();
        let brain = Brain::open(tmp.path()).unwrap();
        for entry in &corpus {
            brain
                .remember(&entry.key, &entry.content, Visibility::Private)
                .unwrap();
        }
        drop(brain);
        let spectral_size = dir_size(tmp.path());
        let vector_raw = 384 * 4 * 1000; // 384 dims * f32 * 1000 docs

        println!(
            "  Spectral (SQLite + Kuzu + fingerprints): {}",
            human_bytes(spectral_size)
        );
        println!(
            "  Vector embeddings (raw, no index):       {}",
            human_bytes(vector_raw)
        );
        println!("  Vector model on disk (bge-small-en-v1.5): ~130 MB");
        println!("  ONNX Runtime binary:                      ~200 MB");
        println!();
        println!("  At 10k memories (projected):");
        println!(
            "    Spectral: ~{} (linear extrapolation)",
            human_bytes(spectral_size * 10)
        );
        println!("    Vector embeddings: {}", human_bytes(384 * 4 * 10_000));
        println!("    Model + runtime overhead remains fixed: ~330 MB");
    }

    // Cold start
    println!("\n--- Cold-start time ---");
    {
        let spectral_times: Vec<Duration> = (0..10)
            .map(|_| {
                let tmp = tempfile::tempdir().unwrap();
                let start = Instant::now();
                let _brain = Brain::open(tmp.path()).unwrap();
                start.elapsed()
            })
            .collect();
        let spectral_median = median_duration(&spectral_times);

        let vector_start = Instant::now();
        let _model = init_embedding_model(false);
        let vector_time = vector_start.elapsed();

        println!(
            "  Spectral Brain::open (empty, median of 10): {:?}",
            spectral_median
        );
        println!(
            "  Vector model init (cached, single run):     {:?}",
            vector_time
        );
    }

    // Per-query encoding cost
    println!("\n--- Per-query encoding cost ---");
    {
        let mut model = init_embedding_model(false);

        let start = Instant::now();
        let iters = 100;
        for _ in 0..iters {
            let _ = model.embed(vec!["benchmark query for encoding cost measurement"], None);
        }
        let per_query_ms = start.elapsed().as_secs_f64() / iters as f64 * 1000.0;

        println!("  Spectral: no query encoding (regex classification + hash lookup)");
        println!("  Vector:   {per_query_ms:.2} ms per query (single encode, BGE-small-en-v1.5)");
    }

    println!("\n  Summary: Spectral has no model dependency (~0 MB baseline).");
    println!("  Vector requires ~330 MB on disk (model + ONNX Runtime) and ~200 MB RSS.\n");
}

// ── Helpers ──────────────────────────────────────────────────────────

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    total += dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn median_duration(times: &[Duration]) -> Duration {
    let mut sorted: Vec<Duration> = times.to_vec();
    sorted.sort();
    sorted[sorted.len() / 2]
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = bench_speed_and_accuracy, bench_operational,
}
criterion_main!(benches);
