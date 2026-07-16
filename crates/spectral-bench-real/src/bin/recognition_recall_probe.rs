//! Does content RECOGNITION (peak-pairs / MinHash) beat FTS as a RETRIEVAL
//! signal? — the empirical test behind the TACT "unlock".
//!
//! Prior audits (SPECTROGRAM_AUDIT.md) diagnosed TACT's metadata fingerprint as
//! "a recognition engine being asked to do recall" and specced the fix: content
//! peak-pair fingerprinting (backlog T3), which the spectral-recognition crate
//! already implements. But nobody measured whether content recognition, pointed
//! at RETRIEVAL, actually beats FTS. This does — head to head, same corpus, on a
//! spectrum of query↔memory relationships, reporting the RANK of the gold answer
//! in each engine. It locates exactly where recognition's power is (and isn't).
//!
//! Deterministic, $0, no LLM. Run: `cargo run -p spectral-bench-real --bin recognition_recall_probe`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig};
use spectral_recognition::{InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine};
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

/// Rank (1-based) of the memory whose key == `want_key` in a hit list; 0 = absent.
fn rank_of(keys: &[String], want_key: &str) -> usize {
    keys.iter().position(|k| k == want_key).map(|p| p + 1).unwrap_or(0)
}

fn main() {
    let brain = open(&std::env::temp_dir().join("spectral-recog-recall"));
    let mut engine = RecognitionEngine::new(
        InMemoryRecognitionStore::default(),
        RecognitionConfig::default(),
    );

    // Corpus: 20 memories. Index i -> key "m{i}". Enrolled in BOTH the brain
    // (FTS) and the recognition engine (content peak-pairs/MinHash).
    let corpus = [
        "I am severely allergic to shellfish and break out in hives",           // 0
        "We booked the anniversary dinner at the rooftop Italian place",        // 1
        "The quarterly budget review moved to the second week of March",        // 2
        "My daughter started kindergarten at Lincoln Elementary this fall",     // 3
        "I switched my morning coffee from dark roast to a light Ethiopian",    // 4
        "The car needs new brake pads before the winter road trip",             // 5
        "I prefer async written updates over synchronous status meetings",      // 6
        "We adopted a rescue beagle named Biscuit from the county shelter",     // 7
        "The staging deploy failed on a missing environment variable",          // 8
        "I run five kilometers along the river every Tuesday and Thursday",     // 9
        "The dentist recommended a night guard for my teeth grinding",          // 10
        "Our flight to Lisbon connects through Madrid with a short layover",    // 11
        "I keep a shellfish reaction epipen in my bag at all times",            // 12  (near-dup topic of 0)
        "The automobile required a fresh set of stopping pads for the trip",    // 13  (pure synonym of 5)
        "Grinding my teeth at night has worn down the enamel, dentist says",    // 14  (reorder/paraphrase of 10)
        "The budget review for Q2 is now the March 10th standing slot",         // 15  (collocation w/ 2)
        "Biscuit the beagle had his first vet visit this week",                 // 16  (entity carryover from 7)
        "I love a light-roast pour-over to start the day",                      // 17  (synonym-ish of 4)
        "Kindergarten drop-off at Lincoln takes twenty minutes each morning",   // 18  (entity carryover from 3)
        "The Lisbon trip itinerary includes two days in Sintra",                // 19  (entity carryover from 11)
    ];
    for (i, c) in corpus.iter().enumerate() {
        brain.remember(&format!("m{i}"), c, Visibility::Private).unwrap();
        engine.enroll(&format!("m{i}"), c).unwrap();
    }

    // (label, query, gold answer index, relationship-class)
    let cases: &[(&str, &str, usize, &str)] = &[
        ("keyword-overlap ", "what is my shellfish allergy situation", 0, "FTS-easy"),
        ("keyword-overlap ", "when is the budget review", 2, "FTS-easy"),
        ("distinct-collocation", "shellfish reaction epipen", 12, "rare-pair"),
        ("distinct-collocation", "Q2 budget review March slot", 15, "rare-pair"),
        ("reorder/paraphrase", "teeth grinding at night wearing down enamel", 14, "reorder"),
        ("near-duplicate  ", "I am allergic to shellfish and get hives", 0, "near-dup"),
        ("entity-carryover", "how is Biscuit the beagle doing", 16, "entity"),
        ("entity-carryover", "Lincoln kindergarten drop-off", 18, "entity"),
        ("entity-carryover", "the Lisbon trip plans", 19, "entity"),
        ("pure-synonym    ", "automobile stopping pads winter", 13, "synonym"),
        ("pure-synonym    ", "light roast pour over coffee", 17, "synonym"),
    ];

    let cfg = RecallTopKConfig { k: 20, ..RecallTopKConfig::default() };

    println!("=== Recognition (content peak-pairs) vs FTS as a retrieval ranker ===\n");
    println!("Rank of the GOLD answer memory in each engine (1=top, 0=not retrieved). Lower is better.\n");
    println!("{:<22} {:<12} {:>8} {:>8}   winner", "case", "class", "FTS", "Recog");
    println!("{}", "-".repeat(66));

    let (mut fts_wins, mut recog_wins, mut ties, mut fts_only, mut recog_only) = (0, 0, 0, 0, 0);
    for (label, query, gold, class) in cases {
        let gold_key = format!("m{gold}");
        // FTS ranking
        let fts_keys: Vec<String> = brain
            .recall_topk_fts(query, &cfg, Visibility::Private)
            .unwrap()
            .into_iter()
            .map(|h| h.key)
            .collect();
        let fts_rank = rank_of(&fts_keys, &gold_key);
        // Recognition ranking
        let recog_keys: Vec<String> = engine
            .recognize(query)
            .unwrap()
            .traces
            .into_iter()
            .map(|t| t.memory_id)
            .collect();
        let recog_rank = rank_of(&recog_keys, &gold_key);

        // "found" = rank in 1..=5 (usable window); compare, treating 0 as worst.
        let score = |r: usize| if r == 0 { usize::MAX } else { r };
        let (fw, rw) = (score(fts_rank), score(recog_rank));
        let winner = if fw == rw {
            ties += 1;
            "tie"
        } else if fw < rw {
            fts_wins += 1;
            if recog_rank == 0 { fts_only += 1; }
            "FTS"
        } else {
            recog_wins += 1;
            if fts_rank == 0 { recog_only += 1; }
            "RECOG"
        };
        let fmt = |r: usize| if r == 0 { "—".to_string() } else { r.to_string() };
        println!(
            "{:<22} {:<12} {:>8} {:>8}   {}",
            label, class, fmt(fts_rank), fmt(recog_rank), winner
        );
    }

    println!("\n{}", "-".repeat(66));
    println!("FTS wins: {fts_wins} (found {fts_only} that recog missed) | Recog wins: {recog_wins} (found {recog_only} that FTS missed) | ties: {ties}");
    println!("\nRead: where recognition WINS is where its power to unlock TACT lives;");
    println!("where it TIES/LOSES vs FTS is where the audit's caveat bites (recall is");
    println!("not recognition's job — both are lexical, FTS's BM25 is already strong).");
    println!("Deterministic, $0. Locates the boundary of the vision empirically.");
}
