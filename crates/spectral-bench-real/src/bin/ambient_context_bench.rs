//! Ambient context boost — the "what the user is doing in the app" signal,
//! measured for the first time.
//!
//! `ambient_boost_for_hit` (wing-match ×1.5, mismatch ×0.7 under strong
//! context) is ON by default in the cascade pipeline, but every bench passed
//! `RecognitionContext::empty()` — so the ambient feedback loop has never
//! fired in a measurement. The claim under test: for an AMBIGUOUS query whose
//! answer differs by activity context ("the notes" while cooking vs while
//! working), does focus_wing flip the top result to the contextually-correct
//! memory, deterministically? Run: `cargo run -p spectral-bench-real --bin ambient_context_bench`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use spectral_graph::cascade_layers::CascadePipelineConfig;
use spectral_graph::RecognitionContext;
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

fn remember_in_wing(brain: &Brain, key: &str, content: &str, wing: &str) {
    brain
        .remember_with(
            key,
            content,
            RememberOpts {
                visibility: Visibility::Private,
                wing: Some(wing.to_string()),
                ..Default::default()
            },
        )
        .unwrap();
}

fn main() {
    let brain = open(&std::env::temp_dir().join("spectral-ambient-context"));

    // Ambiguous vocabulary across three life contexts: each wing has a memory a
    // bare "review the notes" query could mean.
    remember_in_wing(
        &brain,
        "work:notes",
        "The sprint review notes cover the deploy checklist and open bugs",
        "work",
    );
    remember_in_wing(
        &brain,
        "cooking:notes",
        "The recipe review notes say to double the garlic and halve the salt",
        "cooking",
    );
    remember_in_wing(
        &brain,
        "music:notes",
        "The rehearsal review notes mark the tricky notes in the second verse",
        "music",
    );
    // Filler so ranking isn't trivial.
    for (i, (w, c)) in [
        ("work", "Standup moved to ten thirty on Thursdays"),
        (
            "cooking",
            "The cast iron pan needs reseasoning after the tomato sauce",
        ),
        ("music", "New strings arrive Tuesday for the acoustic"),
    ]
    .iter()
    .enumerate()
    {
        remember_in_wing(&brain, &format!("fill{i}"), c, w);
    }

    let query = "review notes";
    let cfg = CascadePipelineConfig::default(); // ambient boost ON by default
    let top_with = |focus: Option<&str>| -> String {
        let mut ctx = RecognitionContext::empty();
        ctx.focus_wing = focus.map(|s| s.to_string());
        let res = brain.recall_cascade(query, &ctx, &cfg).unwrap();
        res.merged_hits
            .first()
            .map(|h| h.key.clone())
            .unwrap_or_default()
    };

    println!("=== Ambient context boost (dormant signal, first measurement) ===\n");
    println!("query: {query:?} — genuinely ambiguous across wings\n");
    let none = top_with(None);
    println!("  no context (empty)      -> top: {none}");
    let mut correct = 0usize;
    for (wing, expect) in [
        ("work", "work:notes"),
        ("cooking", "cooking:notes"),
        ("music", "music:notes"),
    ] {
        let got = top_with(Some(wing));
        let ok = got == expect;
        correct += ok as usize;
        println!(
            "  focus_wing={wing:<8} -> top: {got:<14} (want {expect}) {}",
            if ok { "✓" } else { "✗" }
        );
    }
    // Diagnostic: full ranking for the cooking focus (the failing case).
    let mut ctx = RecognitionContext::empty();
    ctx.focus_wing = Some("cooking".into());
    let res = brain.recall_cascade(query, &ctx, &cfg).unwrap();
    println!("\n  diagnostic (focus=cooking) full ranking:");
    for h in res.merged_hits.iter().take(6) {
        println!(
            "    {:<14} wing={:?} composite={:.3}",
            h.key, h.wing, h.signal_score
        );
    }

    println!("\nverdict: ambient focus context disambiguated {correct}/3 contexts");
    println!(
        "  -> the 'what the user is doing right now' loop {}",
        if correct == 3 {
            "WORKS deterministically: same query, three contexts, three correct answers"
        } else {
            "is partial — inspect boost weights vs FTS score gaps"
        }
    );
    println!("\nDeterministic, $0, no LLM. The consumer just sets focus_wing / recent_activity.");
}
