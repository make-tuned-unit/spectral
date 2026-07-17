//! Associative recall — the TRUE TACT vision, tested. Does spreading activation
//! through EPISODE co-occurrence recover answer memories that FTS structurally
//! cannot find (zero query-word overlap)?
//!
//! TACT's constellation is a co-occurrence graph, but the shipped code collapses
//! it to a popularity degree-count and keys it on broken metadata. The vision it
//! throws away is ASSOCIATIVE recall: FTS finds the seed by words, then activation
//! spreads through associative links to co-occurring memories that share NO words
//! with the query — exactly the vocabulary-gap misses (query "homegrown
//! ingredients" ↔ memory "growing cherry tomatoes, basil, mint"). Episode
//! (same-session) co-occurrence is the cleanest, popularity-bias-free such link.
//!
//! This constructs the archetype: a query-matching BRIDGE memory and a
//! lexically-disjoint ANSWER in the same episode. FTS finds the bridge, misses
//! the answer; episode-expansion recovers it. Deterministic, $0, no LLM.
//! Run: `cargo run -p spectral-bench-real --bin associative_expansion_probe`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig, RememberOpts};
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

fn remember_ep(brain: &Brain, key: &str, content: &str, ep: &str) {
    brain
        .remember_with(
            key,
            content,
            RememberOpts {
                visibility: Visibility::Private,
                episode_id: Some(ep.to_string()),
                ..Default::default()
            },
        )
        .unwrap();
}

fn rank_of(keys: &[String], want: &str) -> usize {
    keys.iter()
        .position(|k| k == want)
        .map(|p| p + 1)
        .unwrap_or(0)
}

fn main() {
    let brain = open(&std::env::temp_dir().join("spectral-assoc-expand"));

    // Each episode is a "session": a bridge memory (query-matchable) + an answer
    // memory that shares NO distinctive words with the query. Plus noise episodes.
    // (episode, key, content)
    let mems = [
        // e1: dinner/garden — the archetype
        (
            "e1",
            "e1_bridge",
            "What should I cook for the dinner party this weekend?",
        ),
        (
            "e1",
            "e1_answer",
            "I have been growing cherry tomatoes, basil, and mint in my backyard",
        ),
        // e2: pet
        (
            "e2",
            "e2_bridge",
            "Any advice for my upcoming vet appointment?",
        ),
        (
            "e2",
            "e2_answer",
            "Biscuit is a rescue beagle who gets anxious around strangers",
        ),
        // e3: commute
        (
            "e3",
            "e3_bridge",
            "Suggest something for my morning commute",
        ),
        (
            "e3",
            "e3_answer",
            "I subscribed to three history podcasts and a language app",
        ),
        // e4: travel
        ("e4", "e4_bridge", "Help me plan activities for the trip"),
        (
            "e4",
            "e4_answer",
            "We are flying to Lisbon and staying two nights in Sintra",
        ),
        // e5: fitness
        (
            "e5",
            "e5_bridge",
            "What should I do for exercise this week?",
        ),
        (
            "e5",
            "e5_answer",
            "My physical therapist cleared me to jog after the knee surgery",
        ),
        // noise episodes (unrelated, add distractors so FTS/expansion isn't trivial)
        (
            "n1",
            "n1_a",
            "The quarterly budget spreadsheet needs review before Friday",
        ),
        (
            "n1",
            "n1_b",
            "I switched banks to get a better savings interest rate",
        ),
        (
            "n2",
            "n2_a",
            "The staging server crashed on a null pointer exception",
        ),
        (
            "n2",
            "n2_b",
            "We migrated the database to a new managed instance",
        ),
        (
            "n3",
            "n3_a",
            "My favorite coffee is a light Ethiopian pour-over",
        ),
        (
            "n3",
            "n3_b",
            "The anniversary dinner reservation is at the rooftop place",
        ),
    ];
    for (ep, key, content) in &mems {
        remember_ep(&brain, key, content, ep);
    }

    // (query, gold answer key) — the query is lexically DISJOINT from the answer,
    // but shares words with the answer's episode bridge memory.
    let cases: &[(&str, &str)] = &[
        (
            "suggest a dinner using my homegrown ingredients",
            "e1_answer",
        ),
        ("what pet care tips do you have for me", "e2_answer"),
        (
            "recommend audio to listen to on my drive to work",
            "e3_answer",
        ),
        ("what are some good vacation ideas for me", "e4_answer"),
        ("give me a workout plan", "e5_answer"),
    ];

    // k=5 over a 16-memory corpus: FTS returns only its top-5 by relevance, so a
    // lexically-disjoint answer (zero query-term match) falls OUTSIDE the window.
    // The test: does episode-expansion from the in-window bridge recover it?
    let cfg = RecallTopKConfig {
        k: 5,
        ..RecallTopKConfig::default()
    };
    const SEED_EPISODES: usize = 5; // expand episodes of the top-N FTS seeds

    println!("=== Associative recall via episode co-occurrence (the TACT vision) ===\n");
    println!("Gold answer shares NO distinctive words with the query. Rank in each (0=missed):\n");
    println!(
        "{:<48} {:>10} {:>14}   recovered?",
        "query", "FTS-only", "FTS+assoc"
    );
    println!("{}", "-".repeat(84));

    let (mut fts_found, mut assoc_found) = (0, 0);
    for (query, gold) in cases {
        // FTS baseline
        let fts_hits = brain
            .recall_topk_fts(query, &cfg, Visibility::Private)
            .unwrap();
        let fts_keys: Vec<String> = fts_hits.iter().map(|h| h.key.clone()).collect();
        let fts_rank = rank_of(&fts_keys, gold);

        // Associative expansion: spread from the top FTS seeds through their
        // episodes (same-session co-occurrence).
        let mut expanded = fts_keys.clone();
        let seed_eps: Vec<String> = fts_hits
            .iter()
            .take(SEED_EPISODES)
            .filter_map(|h| h.episode_id.clone())
            .collect();
        let mut seen_eps = std::collections::HashSet::new();
        for ep in seed_eps {
            if !seen_eps.insert(ep.clone()) {
                continue;
            }
            for m in brain.list_memories_by_episode(&ep).unwrap() {
                if !expanded.contains(&m.key) {
                    expanded.push(m.key);
                }
            }
        }
        let assoc_rank = rank_of(&expanded, gold);

        if fts_rank != 0 {
            fts_found += 1;
        }
        if assoc_rank != 0 {
            assoc_found += 1;
        }
        let recovered = if fts_rank == 0 && assoc_rank != 0 {
            "✅ ASSOC recovered"
        } else if fts_rank != 0 {
            "(FTS already had it)"
        } else {
            "❌ still missed"
        };
        let fmt = |r: usize| {
            if r == 0 {
                "—".to_string()
            } else {
                r.to_string()
            }
        };
        println!(
            "{:<48} {:>10} {:>14}   {}",
            query,
            fmt(fts_rank),
            fmt(assoc_rank),
            recovered
        );
    }

    println!("\n{}", "-".repeat(84));
    println!(
        "answer found — FTS-only: {}/{}   FTS+associative: {}/{}",
        fts_found,
        cases.len(),
        assoc_found,
        cases.len()
    );
    println!("\nIf FTS misses the lexically-disjoint answer but episode-expansion recovers it,");
    println!("that is TACT's true vision working: associative recall bridging the vocabulary");
    println!("gap FTS structurally cannot cross — NOT competing with FTS, completing it.");
    println!("Deterministic, $0.");
}
