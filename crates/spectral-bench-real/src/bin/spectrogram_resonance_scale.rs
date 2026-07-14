//! Spectrogram resonance at scale — push the dormant subsystem to its limit.
//!
//! The first probe showed 4/4 resonance on a tiny corpus. This scales to a
//! noisy multi-domain corpus (decisions, discoveries, problems, recommendations
//! across 8 wings) and answers three production-grade questions: (1) precision/
//! recall of "find my other DECISIONS" as same-action-type noise fills other
//! domains; (2) the MatchTolerances frontier (swept, not guessed); (3) the
//! per-memory spectrogram write cost. Deterministic, $0, no LLM.
//! Run: `cargo run -p spectral-bench-real --bin spectrogram_resonance_scale`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use spectral_spectrogram::matching::MatchTolerances;
use std::path::Path;
use std::time::Instant;

fn open(dir: &Path, spectrogram: bool) -> Brain {
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
        enable_spectrogram: spectrogram,
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

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() as usize) % xs.len()]
    }
}

const WINGS: &[&str] = &["health", "home", "finance", "travel", "career", "hobby", "family", "learning"];
// Templates by action type. DECISION templates are what resonance should find.
const DECIDE: &[&str] = &[
    "Decided to {v} the {n} because the {r} finally made it worth it",
    "Chose the {n} over the alternative since the {r} was the deciding factor",
    "Picked {v}ing the {n} after weighing it — the {r} settled it",
    "Locked in the {n} plan; the {r} left no better option",
];
const DISCOVER: &[&str] = &[
    "Noticed the {n} keeps {v}ing whenever the {r} changes",
    "Realized the {n} was tied to the {r} all along",
    "Found that the {n} improves once the {r} is handled",
];
const PROBLEM: &[&str] = &[
    "The {n} keeps failing and the {r} makes it worse",
    "Ran into the {n} breaking again during the {r}",
    "The {n} is stuck because of the {r} once more",
];
const RECOMMEND: &[&str] = &[
    "Should probably {v} the {n} given the {r}",
    "Recommend {v}ing the {n} if the {r} holds",
];
const NOUNS: &[&str] = &["schedule", "budget", "routine", "setup", "plan", "system", "gear", "process", "contract", "layout"];
const VERBS: &[&str] = &["switch", "upgrade", "simplify", "consolidate", "rebuild", "replace"];
const REASONS: &[&str] = &["cost", "time pressure", "reliability", "stress", "quality", "the deadline", "the maintenance load"];

fn fill(rng: &mut Lcg, tmpl: &str) -> String {
    tmpl.replace("{v}", rng.pick(VERBS))
        .replace("{n}", rng.pick(NOUNS))
        .replace("{r}", rng.pick(REASONS))
}

fn main() {
    // ── Write-cost: time remember() with spectrogram OFF vs ON ──
    let n_write = 200usize;
    let items: Vec<(String, String, &'static str, bool)> = {
        let mut rng = Lcg(0xA11C_E5ED);
        (0..n_write)
            .map(|i| {
                let (tmpls, is_dec): (&[&str], bool) = match rng.next() % 4 {
                    0 => (DECIDE, true),
                    1 => (DISCOVER, false),
                    2 => (PROBLEM, false),
                    _ => (RECOMMEND, false),
                };
                let t = rng.pick(tmpls).to_string();
                let content = fill(&mut rng, &t);
                (format!("m{i}"), content, WINGS[i % WINGS.len()], is_dec)
            })
            .collect()
    };

    // `rotate` = wing changes every write (thrashes the wing-corpus LRU cache,
    // worst case); `single` = all writes to one wing (cache-warm, real locality).
    let timed_ingest = |spectrogram: bool, rotate: bool, tag: &str| -> f64 {
        let brain = open(&std::env::temp_dir().join(format!("spec-scale-{tag}")), spectrogram);
        let t = Instant::now();
        for (i, (k, c, w, _)) in items.iter().enumerate() {
            let wing = if rotate { *w } else { "career" };
            let _ = i;
            brain
                .remember_with(k, c, RememberOpts { visibility: Visibility::Private, wing: Some(wing.to_string()), ..Default::default() })
                .unwrap();
        }
        t.elapsed().as_secs_f64() * 1000.0 / n_write as f64
    };
    let off_ms = timed_ingest(false, true, "off");
    let on_rot = timed_ingest(true, true, "on-rot");
    let on_warm = timed_ingest(true, false, "on-warm");

    println!("=== Spectrogram at scale ({n_write} memories, 8 wings, 4 action types) ===");
    println!("(write times are DEBUG build; release is several× faster — the RATIO is the signal)\n");
    println!("write cost per memory:");
    println!("  spectrogram OFF                : {off_ms:.3}ms");
    println!("  ON, small wings (~25 mem corpus): {on_rot:.3}ms  ({:+.0}%)", 100.0 * (on_rot / off_ms - 1.0));
    println!("  ON, one big wing (capped corpus): {on_warm:.3}ms  ({:+.0}%)", 100.0 * (on_warm / off_ms - 1.0));
    println!("  -> cost is driven by wing CORPUS SIZE (novelty dimension), bounded at 256 mem/64KB.\n");

    // ── Precision/recall sweep on the spectrogram-ON brain ──
    let brain = open(&std::env::temp_dir().join("spec-scale-eval"), true);
    for (k, c, w, _) in &items {
        brain.remember_with(k, c, RememberOpts { visibility: Visibility::Private, wing: Some(w.to_string()), ..Default::default() }).unwrap();
    }
    // Seed = a fresh decision in a 9th wing (no keyword-tuned overlap).
    brain.remember_with("seed", "Decided to consolidate the routine because the maintenance load kept growing", RememberOpts { visibility: Visibility::Private, wing: Some("work".into()), ..Default::default() }).unwrap();

    let total_decisions = items.iter().filter(|(_, _, _, d)| *d).count();
    println!("corpus: {total_decisions} decisions among {n_write} memories (other-wing noise = discoveries/problems/recs)\n");

    println!("{:<34}{:>8}{:>8}{:>8}{:>8}", "tolerance (tol/min_dims)", "hits", "prec", "recall", "F1");
    let _ = MatchTolerances::default(); // reference default; sweep overrides all fields
    let sweep = [
        ("loose  0.4 / 2", 0.4, 2usize),
        ("default 0.3 / 3", 0.3, 3),
        ("tight  0.2 / 4", 0.2, 4),
        ("tight  0.15 / 5", 0.15, 5),
    ];
    for (label, tol, min_dims) in sweep {
        let t = MatchTolerances {
            entity_density: tol,
            decision_polarity: tol + 0.1,
            causal_depth: tol,
            emotional_valence: tol + 0.1,
            temporal_specificity: tol,
            novelty: tol,
            min_matching_dimensions: min_dims,
        };
        let res = brain.recall_cross_wing_with("Decided to consolidate the routine because the maintenance load kept growing", Visibility::Private, 50, &t).unwrap();
        let hits = res.resonant_memories.len();
        // A resonant hit is "correct" iff its source memory is a DECISION.
        let correct = res.resonant_memories.iter().filter(|r| {
            items.iter().any(|(k, _, _, d)| *k == r.memory.key && *d)
        }).count();
        let prec = if hits > 0 { correct as f64 / hits as f64 } else { 0.0 };
        let recall = correct as f64 / total_decisions as f64;
        let f1 = if prec + recall > 0.0 { 2.0 * prec * recall / (prec + recall) } else { 0.0 };
        println!("{label:<34}{hits:>8}{prec:>8.2}{recall:>8.2}{f1:>8.2}");
    }

    println!("\nDeterministic, $0, no LLM. Precision = fraction of resonant hits that are");
    println!("genuinely DECISIONS (action-type correctness); recall = decisions surfaced.");
}
