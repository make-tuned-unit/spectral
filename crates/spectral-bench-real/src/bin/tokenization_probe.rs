//! Tokenization mismatch sweep — hunt for "answer never enters the pool" bugs
//! like the possessive bug (`Marcus's`→`Marcuss`→never matches→answer dropped
//! from the candidate pool entirely). These are the only recall failures that
//! move recall@K (LongMemEval's operating point): getting an answer memory that
//! is currently ABSENT from the pool INTO it — recall EXPANSION, not reordering.
//!
//! For each case: enroll one answer memory + a few distractors, then query with
//! the natural user phrasing whose tokenization may disagree with how FTS5
//! (porter+unicode61) tokenized the stored content. A MISS means the query form
//! silently drops the answer — a possessive-style bug worth a deterministic fix.
//! Runs on the DEFAULT recall path (porter, fusion off). Deterministic, $0.
//!
//! Run: `cargo run -p spectral-bench-real --bin tokenization_probe`

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

struct Case {
    label: &'static str,
    content: &'static str,
    // query forms a user might type; each tested independently.
    queries: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case { label: "possessive (regression)", content: "Marcus got promoted to Director of Engineering", queries: &["Marcus's title", "Marcus title"] },
    Case { label: "hyphenated compound", content: "We shipped the blue-green deploy last night", queries: &["blue-green", "blue green", "bluegreen"] },
    Case { label: "number vs word", content: "The webhook retries five times before failing", queries: &["5 retries", "five retries"] },
    Case { label: "acronym/dotted", content: "The embassy in the U.S. was closed on Monday", queries: &["US embassy", "U.S. embassy"] },
    Case { label: "contraction", content: "The service doesn't restart automatically after a crash", queries: &["doesn't restart", "doesnt restart", "does not restart"] },
    Case { label: "slash pair", content: "The and/or clause caused a parsing ambiguity", queries: &["and/or clause", "and or clause"] },
    Case { label: "compound split", content: "The checkout flow timed out under load", queries: &["checkout flow", "check out flow"] },
    Case { label: "camelCase ident", content: "The taskRunner service crashed at midnight", queries: &["taskRunner", "task runner", "task-runner"] },
    Case { label: "unit-number", content: "Latency spiked to 4200ms during the incident", queries: &["4200ms latency", "4200 ms latency"] },
    Case { label: "dotted hostname", content: "The cert for api.acme.dev expired overnight", queries: &["api.acme.dev cert", "api acme dev cert"] },
    Case { label: "version token", content: "We upgraded to Postgres 16.2 in production", queries: &["Postgres 16.2", "Postgres 16", "postgres v16"] },
    Case { label: "percent/currency", content: "Revenue grew 12% to $4.2M in the quarter", queries: &["12% growth", "12 percent growth", "$4.2M revenue"] },
    Case { label: "email address", content: "Alice at alice@acme.io filed the ticket", queries: &["alice@acme.io", "alice acme.io"] },
    Case { label: "plural (porter baseline)", content: "The database stores customer records", queries: &["databases", "customer record"] },
    // ── chat-relevant classes (accents, numbers, ordinals) ──
    Case { label: "accented name", content: "José reviewed the café renovation plans", queries: &["Jose review", "José review"] },
    Case { label: "diacritic word", content: "Her naïve estimate was off by half", queries: &["naive estimate", "naïve estimate"] },
    Case { label: "umlaut name", content: "Zoë joined the on-call rotation this week", queries: &["Zoe on-call", "Zoë on-call"] },
    Case { label: "number-word count", content: "They adopted three rescue dogs last spring", queries: &["3 dogs", "three dogs"] },
    Case { label: "ordinal", content: "She finished in second place at the regional", queries: &["2nd place", "second place"] },
    Case { label: "hyphenated name", content: "Mary-Jane led the offsite planning", queries: &["Mary-Jane offsite", "Mary Jane offsite"] },
    Case { label: "spelled acronym", content: "The CEO approved the budget increase", queries: &["chief executive", "CEO budget"] },
];

fn main() {
    let brain = open(&std::env::temp_dir().join("spectral-tokenization-probe"));

    // Distractors so retrieval isn't trivial (answer must actually match).
    for (i, d) in [
        "The weekly planning meeting ran long again",
        "Coffee supplies in the kitchen need restocking",
        "The parking garage closes at ten each night",
        "A new hire starts in the design team on Monday",
    ].iter().enumerate() {
        brain.remember(&format!("distractor-{i}"), d, Visibility::Private).unwrap();
    }
    // One answer memory per case, opaque key so the key column can't leak terms.
    for (i, c) in CASES.iter().enumerate() {
        brain.remember(&format!("ans-{i}"), c.content, Visibility::Private).unwrap();
    }

    let retrieves = |query: &str, answer_key: &str| -> bool {
        brain
            .recall_topk_fts(query, &RecallTopKConfig { k: 40, ..Default::default() }, Visibility::Private)
            .unwrap()
            .iter()
            .any(|h| h.key == answer_key)
    };

    println!("=== Tokenization mismatch sweep (default recall path: porter, fusion off) ===");
    println!("MISS = the query form drops the answer from the pool (a possessive-style bug)\n");

    let mut misses: Vec<(String, String)> = Vec::new();
    for (i, c) in CASES.iter().enumerate() {
        let key = format!("ans-{i}");
        print!("{:<26} ", c.label);
        let mut marks = Vec::new();
        for q in c.queries {
            let hit = retrieves(q, &key);
            marks.push(format!("{:>1}{:?}", if hit { "✓" } else { "✗" }, q));
            if !hit {
                misses.push((c.label.to_string(), (*q).to_string()));
            }
        }
        println!("{}", marks.join("  "));
    }

    // ── Number-word bridging (SPECTRAL_NUMBER_NORMALIZE) ──
    // A case where the ONLY discriminating token is the number: content says
    // "three", query says "3". Without bridging the answer is absent from the
    // pool; with it, present. Uses a distinct-vocabulary answer so nothing else
    // matches.
    brain.remember("numbridge", "The household adopted three golden retrievers", Visibility::Private).unwrap();
    // Query noun ("puppies") is NOT in the content, so the ONLY possible bridge
    // to the answer is 3 -> three.
    let bridge_hit = |q: &str| retrieves(q, "numbridge");
    println!("\n--- number-word bridging (query '3 puppies' vs content 'three ...retrievers') ---");
    std::env::remove_var("SPECTRAL_NUMBER_NORMALIZE");
    println!("  OFF: '3 puppies' retrieves answer = {}", bridge_hit("3 puppies"));
    std::env::set_var("SPECTRAL_NUMBER_NORMALIZE", "1");
    println!("  ON:  '3 puppies' retrieves answer = {}", bridge_hit("3 puppies"));
    std::env::remove_var("SPECTRAL_NUMBER_NORMALIZE");

    println!("\n--- MISSES (answer absent from pool) ---");
    if misses.is_empty() {
        println!("  none — every query form retrieved its answer.");
    } else {
        for (label, q) in &misses {
            println!("  [{label}] query {q:?} -> answer NOT retrieved");
        }
        println!("\n{} miss(es). Each is a candidate deterministic normalization fix", misses.len());
        println!("that would EXPAND recall@K (move an answer from absent → present).");
    }
}
