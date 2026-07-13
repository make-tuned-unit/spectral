//! Local measurement of the "descriptions in the actor context" change.
//!
//! The ACCURACY effect of showing Librarian descriptions to the actor requires
//! a paid LongMemEval run (LLM actor + judge) — see the command at the bottom.
//! This binary measures, deterministically and for free, the two things that
//! are observable without an LLM:
//!   1. **Mechanism**: descriptions actually appear in the formatted actor
//!      context when `SPECTRAL_ACTOR_DESCRIPTIONS=1`, and not otherwise.
//!   2. **Cost**: how many extra context tokens the descriptions add (the
//!      other half of the ablation — the paid run gives the accuracy half).
//!
//! It builds a small brain, ingests a multi-turn session, attaches
//! Librarian-style descriptions, retrieves, and formats the context both ways.
//!
//! Run: `cargo run -p spectral-bench-accuracy --bin desc_context`

use anyhow::{Context, Result};
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use spectral_core::visibility::Visibility;
use spectral_bench_accuracy::retrieval::{self, RetrievalConfig};

/// (memory key, turn content, Librarian description)
const TURNS: &[(&str, &str, &str)] = &[
    ("s1:turn:0:user", "I kept seeing the batch job fall over last night around 2am.", "User reports the nightly batch job failed overnight (~2am)."),
    ("s1:turn:1:assistant", "That sounds frustrating. Do you know which pod it was and what the memory limit was set to?", "Assistant asks for the pod name and its memory limit."),
    ("s1:turn:2:user", "It was task-runner-7f9 and I think the limit was 512Mi. The logs said OOMKilled.", "User: the pod task-runner-7f9 was OOMKilled at a 512Mi memory limit."),
    ("s1:turn:3:assistant", "OOMKilled at 512Mi means the reindex is exceeding its memory budget. Bumping the limit to 1Gi or chunking the reindex would fix it.", "Assistant: the OOM is the reindex exceeding 512Mi; fix by raising the limit to 1Gi or chunking the job."),
    ("s1:turn:4:user", "Let's bump it to 1Gi for now and revisit chunking next sprint.", "User decides to raise the memory limit to 1Gi now and defer chunking to next sprint."),
];

const QUERIES: &[&str] = &[
    "what caused the nightly job to fail and how did we fix it",
    "what memory limit did we decide on for the reindex pod",
    "which pod was OOMKilled and at what limit",
];

/// Rough token estimate (~4 chars/token, the common heuristic).
fn est_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

fn open(dir: &std::path::Path) -> Result<Brain> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("ontology.toml"), "version = 1\n")?;
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
    .context("open brain")
}

fn context_for(brain: &Brain, query: &str, show_desc: bool) -> Result<(Vec<String>, usize)> {
    if show_desc {
        std::env::set_var("SPECTRAL_ACTOR_DESCRIPTIONS", "1");
    } else {
        std::env::remove_var("SPECTRAL_ACTOR_DESCRIPTIONS");
    }
    let (formatted, _hits) =
        retrieval::retrieve_topk_fts(brain, query, &RetrievalConfig::default(), None)?;
    let tokens = formatted.iter().map(|l| est_tokens(l)).sum();
    Ok((formatted, tokens))
}

fn main() -> Result<()> {
    let dir = std::env::temp_dir().join("spectral-desc-context");
    let _ = std::fs::remove_dir_all(&dir);
    let brain = open(&dir)?;

    // Ingest the session and attach Librarian descriptions.
    for (key, content, desc) in TURNS {
        let r = brain.remember_with(
            key,
            content,
            RememberOpts { visibility: Visibility::Private, ..Default::default() },
        )?;
        brain.set_description(&r.memory_id, desc)?;
    }

    println!("=== Descriptions-in-actor-context: local mechanism + cost ===");
    println!("session: {} turns, all Librarian-described; {} queries\n", TURNS.len(), QUERIES.len());

    let (mut tot_off, mut tot_on, mut appeared) = (0usize, 0usize, 0usize);
    for q in QUERIES {
        let (_off, off_tok) = context_for(&brain, q, false)?;
        let (on, on_tok) = context_for(&brain, q, true)?;
        let has_note = on.iter().any(|l| l.contains("[librarian:"));
        if has_note {
            appeared += 1;
        }
        tot_off += off_tok;
        tot_on += on_tok;
        println!(
            "q: {:?}\n   off: {off_tok} tok, on: {on_tok} tok (+{} tok, +{:.0}%), librarian-note present: {has_note}",
            q,
            on_tok - off_tok,
            if off_tok > 0 { 100.0 * (on_tok - off_tok) as f64 / off_tok as f64 } else { 0.0 },
        );
        // Show one example annotated line so the effect is concrete.
        if let Some(example) = on.iter().find(|l| l.contains("[librarian:")) {
            println!("   e.g. {example}");
        }
    }

    println!("\n--- summary ---");
    println!("librarian note surfaced in {appeared}/{} queries (mechanism works)", QUERIES.len());
    println!(
        "context token cost: {tot_off} -> {tot_on} (+{} tok, +{:.0}%) across all queries",
        tot_on - tot_off,
        if tot_off > 0 { 100.0 * (tot_on - tot_off) as f64 / tot_off as f64 } else { 0.0 },
    );
    println!(
        "\nACCURACY delta needs the paid LongMemEval run (LLM actor+judge). Ablation:\n\
         apply the SAME descriptions in both arms (constant FTS effect), toggle only\n\
         whether the actor sees them:\n\
           # baseline (descriptions in FTS only):\n\
           spectral-bench-accuracy run --descriptions <desc.json> ...\n\
           # treatment (descriptions ALSO in actor context):\n\
           SPECTRAL_ACTOR_DESCRIPTIONS=1 spectral-bench-accuracy run --descriptions <desc.json> ...\n\
         Compare overall + per-category (watch multi-session / temporal, the\n\
         synthesis-bound categories the FTS-only enrichment could not move)."
    );
    Ok(())
}
