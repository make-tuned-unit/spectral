//! Read-path cost: does recall latency grow with corpus size, and where does
//! it go? Deterministic, $0, no LLM.
//!
//! Writes are the rare operation; recall is the hot one. This measures the
//! three public read paths plus `Brain::open` at increasing corpus sizes, so a
//! superlinear term shows up as growth rather than being averaged away.

use std::time::{Duration, Instant};

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, RecallTopKConfig};
use spectral_graph::cascade_layers::CascadePipelineConfig;
use spectral_graph::RecognitionContext;

const STEPS: &[usize] = &[100, 200, 400, 800];
const QUERIES: &[&str] = &[
    "deployment region halifax",
    "sprint retrospective open bugs",
    "on-call rotation checklist",
    "team notes for the release",
    "what did we decide about staging",
];
const REPS: usize = 12;

fn corpus(n: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| {
            (
                format!("bench:key:{i}"),
                format!(
                    "Memory number {i}: the deployment region is Halifax and the \
                     on-call rotation for sprint {} covers deploy checklist items, \
                     open bugs, and the retrospective notes for team {}.",
                    i % 12,
                    i % 5
                ),
            )
        })
        .collect()
}

fn brain_config(dir: &std::path::Path) -> BrainConfig {
    BrainConfig {
        data_dir: dir.to_path_buf(),
        ontology_path: dir.join("ontology.toml"),
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: spectral_graph::brain::EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    println!("=== Read-path cost vs corpus size (release, $0) ===\n");
    println!(
        "{:>7}  {:>12}  {:>12}  {:>12}  {:>12}  {:>10}",
        "corpus", "topk_fts ms", "cascade ms", "casc+async", "recall ms", "open ms"
    );
    println!(
        "{:>7}  {:>12}  {:>12}  {:>12}",
        "", "", "tact_ms", "fts_direct_ms"
    );

    let mut first: Option<(f64, f64, f64)> = None;
    let mut last = (0.0, 0.0, 0.0);

    for &n in STEPS {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("ontology.toml"), "version = 1\n").unwrap();
        let mut brain = Brain::open(brain_config(tmp.path())).unwrap();
        for (k, c) in &corpus(n) {
            brain.remember(k, c, Visibility::Private).unwrap();
        }

        let topk_cfg = RecallTopKConfig::default();
        let casc_cfg = CascadePipelineConfig::default();
        let ctx = RecognitionContext::empty();

        // Warm the page cache so we measure steady-state, not first-touch I/O.
        for q in QUERIES {
            let _ = brain.recall_topk_fts(q, &topk_cfg, Visibility::Private);
        }

        // Cascade with async write-back ON: recall returns at the retrieval
        // floor instead of blocking on auto-reinforce + event logging.
        let mut t_casc_async = Vec::new();
        brain.set_async_writeback(true);
        for _ in 0..REPS {
            for q in QUERIES {
                let s = Instant::now();
                let _ = brain.recall_cascade(q, &ctx, &casc_cfg);
                t_casc_async.push(s.elapsed().as_secs_f64() * 1000.0);
            }
        }
        brain.set_async_writeback(false);

        // Split the cascade's candidate-gathering: TACT tiers vs raw FTS.
        // Both are public Brain methods, so this needs no library change.
        let mut t_tact = Vec::new();
        let mut t_ftsd = Vec::new();
        for _ in 0..REPS {
            for q in QUERIES {
                let s = Instant::now();
                let _ = brain.tact_retrieve_with_k(q, 40);
                t_tact.push(s.elapsed().as_secs_f64() * 1000.0);

                let words: Vec<String> = q
                    .split_whitespace()
                    .filter(|w| w.len() > 2)
                    .map(|w| w.to_lowercase())
                    .collect();
                let s = Instant::now();
                let _ = brain.fts_search_direct(&words, 40);
                t_ftsd.push(s.elapsed().as_secs_f64() * 1000.0);
            }
        }

        let mut t_topk = Vec::new();
        let mut t_casc = Vec::new();
        let mut t_recall = Vec::new();
        for _ in 0..REPS {
            for q in QUERIES {
                let s = Instant::now();
                let _ = brain.recall_topk_fts(q, &topk_cfg, Visibility::Private);
                t_topk.push(s.elapsed().as_secs_f64() * 1000.0);

                let s = Instant::now();
                let _ = brain.recall_cascade(q, &ctx, &casc_cfg);
                t_casc.push(s.elapsed().as_secs_f64() * 1000.0);

                let s = Instant::now();
                let _ = brain.recall_local(q);
                t_recall.push(s.elapsed().as_secs_f64() * 1000.0);
            }
        }

        drop(brain);
        let mut t_open = Vec::new();
        for _ in 0..5 {
            let s = Instant::now();
            let b = Brain::open(brain_config(tmp.path())).unwrap();
            t_open.push(s.elapsed().as_secs_f64() * 1000.0);
            drop(b);
        }

        let ca = median(t_casc_async);
        let (tt, tf) = (median(t_tact), median(t_ftsd));
        let (a, b, c) = (median(t_topk), median(t_casc), median(t_recall));
        let o: Duration = Duration::from_secs_f64(median(t_open) / 1000.0);
        println!(
            "{:>7}  {:>12.3}  {:>12.3}  {:>12.3}  {:>12.3}  {:>10.2}",
            n,
            a,
            b,
            ca,
            c,
            o.as_secs_f64() * 1000.0
        );
        println!(
            "{:>7}  {:>12}  {:>12.3}  {:>12.3}  (candidate sources)",
            "", "  tact:", tt, tf
        );
        if first.is_none() {
            first = Some((a, b, c));
        }
        last = (a, b, c);
    }

    let f = first.unwrap();
    println!(
        "\ngrowth {}x -> {}x corpus:  topk_fts {:.1}x   cascade {:.1}x   recall_local {:.1}x",
        STEPS[0],
        STEPS[STEPS.len() - 1],
        last.0 / f.0,
        last.1 / f.1,
        last.2 / f.2
    );
    println!("(linear growth here would mean recall degrades as a brain fills up)");
}
