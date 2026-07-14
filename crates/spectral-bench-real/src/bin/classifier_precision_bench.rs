//! Classifier/signal precision under adversarial negatives — failure analysis
//! for the durable-fact patterns that activate AAAK.
//!
//! The durable-fact classifier + signal boosters (constraint/preference/identity/
//! rule) were additive and could OVER-match: "I never got the email" looks like
//! a rule, "the vegan cafe was packed" trips the constraint booster, "I want
//! coffee now" looks like a preference. A false positive here pollutes the
//! always-in-prompt AAAK layer with ephemeral noise — worse than a miss.
//!
//! This labels a set as durable (should reach the AAAK bar: hall∈{fact,
//! preference,rule,decision} AND signal≥0.7) vs ephemeral (must NOT), and
//! reports precision/recall + every misclassification for iteration.
//! Deterministic, $0, no LLM. Run: `cargo run -p spectral-bench-real --bin classifier_precision_bench`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
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

fn id_of(k: &str) -> String {
    format!("{:016x}", u64::from_be_bytes(blake3::hash(k.as_bytes()).as_bytes()[..8].try_into().unwrap()))
}

fn main() {
    let brain = open(&std::env::temp_dir().join("spectral-classifier-prec"));

    // (content, is_durable) — durable = SHOULD reach the AAAK always-in-prompt bar.
    let cases: &[(&str, bool)] = &[
        // ── durable personal facts (true positives) ──
        ("I am vegetarian and do not eat any meat or fish", true),
        ("I am severely allergic to shellfish and peanuts", true),
        ("Never schedule anything for me before 9am", true),
        ("My daughter Mia is five years old", true),
        ("I prefer concise written summaries over long meetings", true),
        ("I'm diabetic and monitor my blood sugar closely", true),
        ("My wife is a cardiologist at the county hospital", true),
        ("I always run my tests before every deploy", true),
        ("I decided to standardize all projects on Rust", true),
        ("I strongly prefer async communication over calls", true),
        // ── ephemeral / transient (hard negatives — must NOT reach the bar) ──
        ("I never got the confirmation email for the order", false),
        ("The vegan cafe downtown was packed at lunch today", false),
        ("I want a coffee right now before the meeting", false),
        ("I am a bit tired today after the long flight", false),
        ("The gluten-free options were limited at the venue", false),
        ("I like how the demo turned out this afternoon", false),
        ("There is always something breaking in prod lately", false),
        ("My son forgot his lunch again this morning", false),
        ("I love that this sprint is finally wrapping up", false),
        ("We must ship the hotfix before the standup", false),
        ("Grabbed a sandwich and answered a few emails", false),
        ("The allergy season has been rough on the whole team", false),
    ];

    for (i, (c, _)) in cases.iter().enumerate() {
        brain.remember(&format!("c{i}"), c, Visibility::Private).unwrap();
    }

    let durable_halls = ["fact", "preference", "rule", "decision"];
    let reaches_bar = |k: &str| -> (bool, String, f64) {
        let m = brain.get_memory(&id_of(k)).ok().flatten().unwrap();
        let hall = m.hall.clone().unwrap_or_default();
        let ok = durable_halls.contains(&hall.as_str()) && m.signal_score >= 0.7;
        (ok, hall, m.signal_score)
    };

    println!("=== Classifier/signal precision under adversarial negatives ===\n");
    let (mut tp, mut fn_, mut fp, mut tn) = (0, 0, 0, 0);
    let mut false_pos: Vec<(&str, String, f64)> = Vec::new();
    let mut false_neg: Vec<(&str, String, f64)> = Vec::new();
    for (i, (c, durable)) in cases.iter().enumerate() {
        let (bar, hall, sig) = reaches_bar(&format!("c{i}"));
        match (*durable, bar) {
            (true, true) => tp += 1,
            (true, false) => { fn_ += 1; false_neg.push((c, hall, sig)); }
            (false, true) => { fp += 1; false_pos.push((c, hall, sig)); }
            (false, false) => tn += 1,
        }
    }
    let n_dur = cases.iter().filter(|(_, d)| *d).count();
    let n_eph = cases.len() - n_dur;
    let prec = tp as f64 / (tp + fp).max(1) as f64;
    let recall = tp as f64 / n_dur as f64;
    println!("durable (n={n_dur}): {tp} reached bar (recall {recall:.2}), {fn_} missed");
    println!("ephemeral (n={n_eph}): {tn} correctly excluded, {fp} FALSE POSITIVES (precision {prec:.2})\n");

    if !false_pos.is_empty() {
        println!("FALSE POSITIVES (ephemeral wrongly reaching AAAK — pollute the prompt):");
        for (c, h, s) in &false_pos {
            println!("  [{h:<10} {s:.2}] {c}");
        }
    }
    if !false_neg.is_empty() {
        println!("\nFALSE NEGATIVES (durable facts missed):");
        for (c, h, s) in &false_neg {
            println!("  [{h:<10} {s:.2}] {c}");
        }
    }
    println!("\nDeterministic, $0. Precision on the AAAK bar is what matters most —");
    println!("a false positive is always-in-prompt noise, worse than a miss.");
}
