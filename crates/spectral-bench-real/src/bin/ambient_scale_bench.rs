//! Ambient feedback at scale — does lift-based anticipatory recall keep beating
//! raw co-count, and keep suppressing the popularity trap, on a LARGE
//! co-retrieval graph?
//!
//! anticipatory_bench proves the mechanism on a handful of memories. This builds
//! 12 topics × 8 memories + one globally-popular memory, drives realistic usage
//! (each topic co-retrieved within itself; the popular item co-retrieved across
//! everything), rebuilds the co-retrieval index, then for every topic seed
//! compares `recommend` (lift) vs `related_memories` (raw co_count) on two
//! metrics: precision@5 (fraction of recommendations that are the seed's OWN
//! topic) and the popularity trap (how often the globally-popular memory is
//! recommended). Deterministic ($0, no LLM).
//!
//! Run: `cargo run -p spectral-bench-real --bin ambient_scale_bench`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig};
use std::path::Path;

fn open(dir: &Path) -> Brain {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("ontology.toml"), "version = 1\n").unwrap();
    Brain::open(BrainConfig {
        data_dir: dir.to_path_buf(),
        ontology_path: dir.join("ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
    })
    .unwrap()
}

const TOPICS: usize = 12;
const PER_TOPIC: usize = 8;

// Distinctive per-topic subject so a topic query co-retrieves its own members,
// plus shared broad words ("review notes") so a broad query spans topics.
const SUBJECTS: &[&str] = &["kubernetes", "billing", "onboarding", "search", "payments", "telemetry", "caching", "auth", "mobile", "reporting", "ingestion", "scheduling"];
const DETAILS: &[&str] = &["rollout plan", "cost review", "incident recap", "design tradeoff", "capacity limit", "migration step", "config change", "latency budget"];

fn recall(brain: &Brain, q: &str) {
    let _ = brain.recall_topk_fts(q, &RecallTopKConfig::default(), Visibility::Private);
}

fn main() {
    let brain = open(&std::env::temp_dir().join("spectral-ambient-scale"));

    // Enroll topics. Each memory shares its topic subject + the broad words
    // "review notes" (so a broad query can span topics and build the trap).
    let mut seed_ids: Vec<String> = Vec::with_capacity(TOPICS);
    for (t, subject) in SUBJECTS.iter().enumerate() {
        for (i, detail) in DETAILS.iter().enumerate() {
            let key = format!("t{t:02}m{i}");
            let content = format!("{subject} {detail} review notes for item {i}");
            let id = brain.remember(&key, &content, Visibility::Private).unwrap().memory_id;
            if i == 0 {
                seed_ids.push(id);
            }
        }
    }
    // Globally-popular memory: matches the broad "review notes" query strongly
    // (short, high broad-term density), so it is co-retrieved across many topics
    // — high raw co_count with everything, but specifically associated with none.
    brain.remember("popular", "weekly review notes summary", Visibility::Private).unwrap();

    // ── Drive usage ──
    // Within-topic association: each topic subject query co-retrieves its members.
    for subject in SUBJECTS {
        for _ in 0..6 {
            recall(&brain, subject);
        }
    }
    // Broad usage: the popular memory rides along with a wide spread of items.
    for _ in 0..30 {
        recall(&brain, "weekly review notes");
    }
    // Diffuse popularity trap: `DETAILS[0]` ("rollout plan") is item 0 of EVERY
    // topic, so this query co-retrieves all 12 topic seeds together with the
    // popular memory, repeatedly. Now each seed's raw co_count with the popular
    // memory (and with other-topic seeds) EXCEEDS its co_count with its own
    // siblings — the exact bias that makes raw co-count recommend the wrong,
    // globally-popular thing. Lift should divide out the popular memory's high
    // occurrence and still rank the seed's genuine same-topic siblings first.
    for _ in 0..20 {
        recall(&brain, "rollout plan weekly review notes");
    }
    let pairs = brain.rebuild_co_retrieval_index().unwrap();
    println!("=== Ambient feedback at scale (lift vs raw co-count) ===");
    println!("{} topics × {} memories + 1 popular; co_retrieval_pairs = {pairs}\n", TOPICS, PER_TOPIC);

    let topic_of = |key: &str| -> Option<usize> {
        // keys look like "t03m5"
        key.strip_prefix('t').and_then(|r| r.get(0..2)).and_then(|s| s.parse::<usize>().ok())
    };

    let (mut lift_prec, mut cc_prec) = (0.0f64, 0.0f64);
    let (mut lift_pop, mut cc_pop) = (0usize, 0usize);
    let mut seeds = 0usize;
    for (t, seed_id) in seed_ids.iter().enumerate() {
        seeds += 1;

        let lift = brain.recommend(seed_id, 5, 1).unwrap();
        let cc = brain.related_memories(seed_id, 5).unwrap();

        let key_of = |id: &str| brain.get_memory(id).ok().flatten().map(|m| m.key).unwrap_or_default();

        let score = |recs: &[spectral_ingest::RelatedMemory]| -> (f64, bool) {
            if recs.is_empty() {
                return (0.0, false);
            }
            let mut same = 0usize;
            let mut pop = false;
            for r in recs {
                let k = key_of(&r.memory_id);
                if k == "popular" {
                    pop = true;
                }
                if topic_of(&k) == Some(t) {
                    same += 1;
                }
            }
            (same as f64 / recs.len() as f64, pop)
        };

        let (lp, lpop) = score(&lift);
        let (cp, cpop) = score(&cc);
        lift_prec += lp;
        cc_prec += cp;
        lift_pop += lpop as usize;
        cc_pop += cpop as usize;
    }

    let n = seeds as f64;
    println!("aggregate over {seeds} topic seeds:");
    println!("{:<28}{:>14}{:>14}", "metric", "lift (recommend)", "co_count (related)");
    println!("{:<28}{:>14.2}{:>14.2}", "precision@5 (same topic)", lift_prec / n, cc_prec / n);
    println!("{:<28}{:>14}{:>14}", "popular in top-5 (count)", lift_pop, cc_pop);
    println!("\nHigher precision + lower popular-count = better anticipatory targeting.");
    println!("Lift should keep the popular memory out of recommendations at scale.");
}

