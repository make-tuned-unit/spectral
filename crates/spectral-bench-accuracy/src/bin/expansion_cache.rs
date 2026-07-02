//! Generate a frozen query-expansion cache: {question_id: expanded_query}.
//!
//! One Haiku call per question (~$0.12 for 500). The cache makes the
//! oracle's expansion-ON configuration replayable forever at $0 via
//! `oracle --expansion-cache`.
//!
//! Usage: expansion_cache --dataset <path> [--output expansion-cache.json]

use anyhow::{Context, Result};
use spectral_bench_accuracy::expansion::{expand_query, ExpansionConfig};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dataset = args
        .iter()
        .position(|a| a == "--dataset")
        .and_then(|i| args.get(i + 1))
        .context("--dataset <path> required")?;
    let output = args
        .iter()
        .position(|a| a == "--output")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "expansion-cache.json".to_string());

    let api_key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY not set")?;
    let config = ExpansionConfig {
        enabled: true,
        model: "claude-haiku-4-5-20251001".into(),
        base_url: "https://api.anthropic.com".into(),
        api_key,
        max_terms: 10,
    };

    let ds = spectral_bench_accuracy::dataset::load_dataset(std::path::Path::new(dataset))?;

    // Resume support: load existing cache, skip already-expanded questions.
    let mut cache: std::collections::BTreeMap<String, String> =
        std::fs::read_to_string(&output)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    eprintln!("{} questions, {} already cached", ds.len(), cache.len());

    let mut in_tok = 0u64;
    let mut out_tok = 0u64;
    let mut failures = 0usize;
    for (i, q) in ds.iter().enumerate() {
        if cache.contains_key(&q.question_id) {
            continue;
        }
        match expand_query(&q.question, &config) {
            Ok((expanded, usage)) => {
                if let Some(u) = usage {
                    in_tok += u.input_tokens.unwrap_or(0);
                    out_tok += u.output_tokens.unwrap_or(0);
                }
                cache.insert(q.question_id.clone(), expanded);
            }
            Err(e) => {
                eprintln!("  [{}] {} expansion failed: {e}", i, q.question_id);
                failures += 1;
                if failures > 20 {
                    anyhow::bail!("too many failures; aborting (cache saved)");
                }
            }
        }
        // Durable checkpoint every 25 questions.
        if i % 25 == 0 {
            std::fs::write(&output, serde_json::to_string_pretty(&cache)?)?;
            eprint!("\r{}/{} cached", cache.len(), ds.len());
        }
    }
    std::fs::write(&output, serde_json::to_string_pretty(&cache)?)?;

    // Haiku 4.5: $0.80/MTok in, $4.00/MTok out.
    let cost = in_tok as f64 * 0.80 / 1e6 + out_tok as f64 * 4.00 / 1e6;
    eprintln!(
        "\ndone: {} cached, {} failures, tokens {}in/{}out, cost ${cost:.3}",
        cache.len(),
        failures,
        in_tok,
        out_tok
    );
    Ok(())
}
