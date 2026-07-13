//! Reproduce and diagnose the emb-promo retrieval miss: a 6-turn session where
//! the query "What is Marcus's new job title?" retrieved assistant turns but
//! MISSED the user turn that states "Marcus got bumped up to Director of
//! Engineering". Deterministic, $0 — no LLM.

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig, RememberOpts};
use std::path::Path;

const TURNS: &[(&str, &str)] = &[
    ("s2:turn:0:user", "Rough week at work. The Q2 roadmap got completely reshuffled and nobody told the team until the last minute."),
    ("s2:turn:1:assistant", "That kind of last-minute change is demoralizing. Was there a reason given?"),
    ("s2:turn:2:user", "Reorg. Half the leads got shuffled. Marcus got bumped up to Director of Engineering, which honestly he earned, but it left our squad without a lead."),
    ("s2:turn:3:assistant", "Congrats to Marcus, but a leaderless squad mid-roadmap is tough. Is there an interim plan?"),
    ("s2:turn:4:user", "Not yet. I might have to step up informally, which I don't love given the workload."),
    ("s2:turn:5:assistant", "Stepping up informally without the title or pay is a common trap. Worth raising with your manager."),
];
const QUERY: &str = "What is Marcus's new job title?";

fn open(dir: &Path) -> Brain {
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

fn main() {
    let dir = std::env::temp_dir().join("spectral-recall-debug");
    let _ = std::fs::remove_dir_all(&dir);
    let brain = open(&dir);
    for (key, content) in TURNS {
        brain
            .remember_with(key, content, RememberOpts { visibility: Visibility::Private, ..Default::default() })
            .unwrap();
    }

    println!("=== recall_debug: emb-promo miss ===");
    println!("query: {QUERY:?}\n");

    // Full-pool recall (k huge so K-truncation is NOT the cause).
    let cfg = RecallTopKConfig { k: 50, fetch_mult: 1, apply_recency_weighting: false, ..Default::default() };
    let hits = brain.recall_topk_fts(QUERY, &cfg, Visibility::Private).unwrap();
    println!("recall_topk_fts returned {} hits:", hits.len());
    for h in &hits {
        println!("  {:<22} signal={:.2}  {}", h.key, h.signal_score, &h.content[..h.content.len().min(60)]);
    }
    let got_answer = hits.iter().any(|h| h.key == "s2:turn:2:user");
    println!("\nanswer turn (s2:turn:2:user, 'Marcus ... Director of Engineering') retrieved: {got_answer}");

    // Isolate: which turns literally contain the salient query token "marcus"?
    println!("\nturns containing 'marcus' (lowercased):");
    for (key, content) in TURNS {
        if content.to_lowercase().contains("marcus") {
            println!("  {key}");
        }
    }

    // Ablate re-ranking signals one at a time to find which drops the turn.
    println!("\n--- ablation: does a re-ranking stage drop the answer turn? ---");
    let probe = |label: &str, c: RecallTopKConfig| {
        let h = brain.recall_topk_fts(QUERY, &c, Visibility::Private).unwrap();
        let present = h.iter().any(|x| x.key == "s2:turn:2:user");
        println!("  {label:<28} answer_present={present}  n={}", h.len());
    };
    probe("all signals default", RecallTopKConfig { k: 50, ..Default::default() });
    probe("no signal_score weighting", RecallTopKConfig { k: 50, apply_signal_score_weighting: false, ..Default::default() });
    probe("no entity resolution", RecallTopKConfig { k: 50, apply_entity_resolution: false, ..Default::default() });
    probe("no context dedup", RecallTopKConfig { k: 50, apply_context_dedup: false, ..Default::default() });
    probe("no recency", RecallTopKConfig { k: 50, apply_recency_weighting: false, ..Default::default() });

    // ── Stopword filtering: noise reduction without dropping the answer ──
    println!("\n--- stopword filtering (SPECTRAL_FTS_STOPWORDS) ---");
    let show = |label: &str| {
        let h = brain.recall_topk_fts(QUERY, &RecallTopKConfig { k: 50, ..Default::default() }, Visibility::Private).unwrap();
        let keys: Vec<&str> = h.iter().map(|x| x.key.as_str()).collect();
        let answer = keys.contains(&"s2:turn:2:user");
        // turn 1 only matches the stopword "is" — it is pure noise for this query.
        let noise_turn1 = keys.contains(&"s2:turn:1:assistant");
        println!("  {label:<12} n={} answer_kept={answer} noise(turn1_only_'is')={noise_turn1}  {keys:?}", h.len());
    };
    std::env::remove_var("SPECTRAL_FTS_STOPWORDS");
    show("OFF");
    std::env::set_var("SPECTRAL_FTS_STOPWORDS", "1");
    show("ON");
}
