//! Ambient boost weight sweep — push the ambient loop to its limit, honestly.
//!
//! The first ambient measurement showed the default ×1.5/×0.7 weights lose a
//! close call when the out-of-focus competitor has a term-frequency lead. But
//! stronger weights carry a risk: wrongly overriding a STRONG relevance signal
//! (the user explicitly asks for something outside their current context — the
//! query must still win). This sweeps `wing_match` × `mismatch_penalty` over
//! two scenario families and maps the frontier:
//!   A) ambiguity: bare query, answer depends on focus — context SHOULD decide
//!      (including hard cases where the in-focus target has a term-frequency
//!      disadvantage);
//!   B) explicit override: query names an out-of-focus memory unambiguously —
//!      context must NOT hijack it.
//! Score = A-rate (disambiguation) and B-rate (respect-explicit). The right
//! default maximizes A subject to B = 100%. Deterministic, $0, no LLM.
//!
//! Run: `cargo run -p spectral-bench-real --bin ambient_weight_sweep`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use spectral_graph::cascade_layers::{AmbientBoostWeights, CascadePipelineConfig};
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

fn rw(brain: &Brain, key: &str, content: &str, wing: &str) {
    brain
        .remember_with(key, content, RememberOpts {
            visibility: Visibility::Private,
            wing: Some(wing.to_string()),
            ..Default::default()
        })
        .unwrap();
}

struct AmbCase {
    query: &'static str,
    focus: &'static str,
    expect: &'static str,
}

fn main() {
    let brain = open(&std::env::temp_dir().join("spectral-ambient-sweep"));

    // ── Corpus: 4 ambiguity groups across 3 wings each ──
    // Group "notes": music has DOUBLE the query term (hard case for context).
    rw(&brain, "work:notes",    "The sprint review notes cover the deploy checklist and open bugs", "work");
    rw(&brain, "cooking:notes", "The recipe review notes say to double the garlic and halve the salt", "cooking");
    rw(&brain, "music:notes",   "The rehearsal review notes mark the tricky notes in the second verse", "music");
    // Group "list": cooking has the term-frequency lead.
    rw(&brain, "work:list",    "The onboarding list tracks laptop, badge, and account setup", "work");
    rw(&brain, "cooking:list", "The shopping list lists tomatoes, basil, and the list of spices", "cooking");
    rw(&brain, "music:list",   "The setlist list orders the opening songs for Friday", "music");
    // Group "schedule": work has the lead.
    rw(&brain, "work:sched",    "The release schedule schedules the deploy schedule for Thursday", "work");
    rw(&brain, "cooking:sched", "The meal schedule plans dinners through Sunday", "cooking");
    rw(&brain, "music:sched",   "The practice schedule blocks an hour after work", "music");
    // Group "budget": music has the lead.
    rw(&brain, "work:budget",    "The team budget covers two conference trips", "work");
    rw(&brain, "cooking:budget", "The grocery budget caps the weekly shop at ninety", "cooking");
    rw(&brain, "music:budget",   "The gear budget budgets the budget for a new amp", "music");

    // ── A) ambiguity cases: focus should decide (12 = 4 groups × 3 wings) ──
    let amb: Vec<AmbCase> = vec![
        AmbCase { query: "review notes", focus: "work",    expect: "work:notes" },
        AmbCase { query: "review notes", focus: "cooking", expect: "cooking:notes" },
        AmbCase { query: "review notes", focus: "music",   expect: "music:notes" },
        AmbCase { query: "the list",     focus: "work",    expect: "work:list" },
        AmbCase { query: "the list",     focus: "cooking", expect: "cooking:list" },
        AmbCase { query: "the list",     focus: "music",   expect: "music:list" },
        AmbCase { query: "the schedule", focus: "work",    expect: "work:sched" },
        AmbCase { query: "the schedule", focus: "cooking", expect: "cooking:sched" },
        AmbCase { query: "the schedule", focus: "music",   expect: "music:sched" },
        AmbCase { query: "the budget",   focus: "work",    expect: "work:budget" },
        AmbCase { query: "the budget",   focus: "cooking", expect: "cooking:budget" },
        AmbCase { query: "the budget",   focus: "music",   expect: "music:budget" },
    ];
    // ── B) explicit-override cases: query names an out-of-focus memory; the
    //      focus must NOT hijack it (6 cases). ──
    let overrides: Vec<AmbCase> = vec![
        AmbCase { query: "sprint deploy checklist bugs",   focus: "cooking", expect: "work:notes" },
        AmbCase { query: "garlic salt recipe",             focus: "work",    expect: "cooking:notes" },
        AmbCase { query: "tomatoes basil spices shopping", focus: "music",   expect: "cooking:list" },
        AmbCase { query: "opening songs setlist friday",   focus: "work",    expect: "music:list" },
        AmbCase { query: "release deploy thursday",        focus: "music",   expect: "work:sched" },
        AmbCase { query: "amp gear",                       focus: "cooking", expect: "music:budget" },
    ];

    let run = |cases: &[AmbCase], w: AmbientBoostWeights| -> usize {
        let cfg = CascadePipelineConfig { ambient_weights: w, ..Default::default() };
        cases
            .iter()
            .filter(|c| {
                let mut ctx = RecognitionContext::empty();
                ctx.focus_wing = Some(c.focus.to_string());
                let res = brain.recall_cascade(c.query, &ctx, &cfg).unwrap();
                res.merged_hits.first().map(|h| h.key == c.expect).unwrap_or(false)
            })
            .count()
    };

    println!("=== Ambient weight sweep: disambiguation (A, n={}) vs explicit-override respect (B, n={}) ===\n", amb.len(), overrides.len());
    println!("{:<22}{:>8}{:>8}{:>10}", "wing_match/mismatch", "A", "B", "verdict");
    let mut best: Option<(f64, f64, usize, usize)> = None;
    for wm in [1.0, 1.25, 1.5, 2.0, 2.5, 3.0] {
        for mp in [1.0, 0.85, 0.7, 0.5, 0.35] {
            let w = AmbientBoostWeights {
                wing_match: wm,
                mismatch_penalty: mp,
                // widen the clamp so large ratios aren't silently capped
                clamp_min: 0.2,
                clamp_max: 4.0,
                ..Default::default()
            };
            let a = run(&amb, w);
            let b = run(&overrides, w);
            let tag = if b == overrides.len() && best.map(|(_, _, ba, _)| a > ba).unwrap_or(true) {
                best = Some((wm, mp, a, b));
                "  <- best so far"
            } else {
                ""
            };
            println!("{:<22}{:>5}/{:<2}{:>5}/{:<2}{:>12}", format!("{wm}/{mp}"), a, amb.len(), b, overrides.len(), tag);
        }
    }
    if let Some((wm, mp, a, b)) = best {
        println!("\nfrontier point: wing_match={wm} mismatch={mp} -> A={a}/{} with B={b}/{} (no explicit-override errors)", amb.len(), overrides.len());
        let d = AmbientBoostWeights::default();
        let da = run(&amb, d);
        let db = run(&overrides, d);
        println!("shipped default (1.5/0.7): A={da}/{} B={db}/{}", amb.len(), overrides.len());
    }
    println!("\nDeterministic, $0, no LLM. The right default maximizes disambiguation subject");
    println!("to never hijacking an explicit query.");
}
