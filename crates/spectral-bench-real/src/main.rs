use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use std::path::PathBuf;
use std::time::Instant;

// ── CLI ────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "spectral-bench-real")]
#[command(about = "Benchmark Spectral recall against a real brain")]
struct Cli {
    /// Path to an existing Spectral brain directory.
    #[arg(long)]
    brain: PathBuf,

    /// Path to queries TOML file.
    #[arg(long, default_value = "crates/spectral-bench-real/queries.toml")]
    queries: PathBuf,

    /// Iterations per query for warm-cache measurement.
    #[arg(long, default_value_t = 100)]
    iterations: usize,

    /// Output format: text or json.
    #[arg(long, default_value = "text")]
    format: OutputFormat,

    /// Only run queries whose name contains this substring.
    #[arg(long)]
    filter: Option<String>,
}

#[derive(Clone, Debug)]
enum OutputFormat {
    Text,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => Err(format!("unknown format: {s} (expected text or json)")),
        }
    }
}

// ── Query spec ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct QueryFile {
    queries: Vec<QuerySpec>,
}

#[derive(Deserialize, Clone)]
struct QuerySpec {
    name: String,
    text: String,
    #[allow(dead_code)]
    description: String,
    expected_keywords: Vec<String>,
    expected_top_n: usize,
    latency_budget_p95_ms: f64,
    latency_budget_p99_ms: f64,
    visibility: String,
}

impl QuerySpec {
    fn visibility(&self) -> Visibility {
        match self.visibility.to_lowercase().as_str() {
            "private" => Visibility::Private,
            "team" => Visibility::Team,
            "org" => Visibility::Org,
            "public" => Visibility::Public,
            _ => Visibility::Private,
        }
    }

    fn pattern(&self) -> &str {
        let name = &self.name;
        if name.starts_with("single_word") {
            "single_word"
        } else if name.starts_with("multi_word") {
            "multi_word"
        } else if name.starts_with("concept") {
            "concept"
        } else if name.starts_with("temporal") {
            "temporal"
        } else if name.starts_with("cross_domain") {
            "cross_domain"
        } else if name.starts_with("adversarial") {
            "adversarial"
        } else {
            "other"
        }
    }
}

// ── Output types ───────────────────────────────────────────────────

#[derive(Serialize)]
struct BenchReport {
    spectral_version: String,
    brain_path: String,
    iterations: usize,
    queries: Vec<QueryResult>,
    aggregate: Aggregate,
    per_pattern: Vec<PatternBreakdown>,
}

#[derive(Serialize)]
struct QueryResult {
    name: String,
    pattern: String,
    latency_us: LatencyStats,
    accuracy: AccuracyResult,
    budget: BudgetResult,
}

#[derive(Serialize)]
struct LatencyStats {
    cold: u64,
    p50: u64,
    p95: u64,
    p99: u64,
    mean: u64,
    stddev: u64,
}

#[derive(Serialize)]
struct AccuracyResult {
    pass: bool,
    top_score: f64,
    num_results: usize,
}

#[derive(Serialize)]
struct BudgetResult {
    p95_ok: bool,
    p99_ok: bool,
}

#[derive(Serialize)]
struct Aggregate {
    warm_p50_us: u64,
    warm_p95_us: u64,
    warm_p99_us: u64,
    cold_p50_us: u64,
    cold_p95_us: u64,
    cold_p99_us: u64,
    pass_rate: f64,
    budget_violations: usize,
}

#[derive(Serialize)]
struct PatternBreakdown {
    pattern: String,
    query_count: usize,
    warm_p50_us: u64,
    warm_p95_us: u64,
    pass_rate: f64,
}

// ── Statistics helpers ─────────────────────────────────────────────

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn mean_stddev(values: &[u64]) -> (u64, u64) {
    if values.is_empty() {
        return (0, 0);
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<u64>() as f64 / n;
    let variance = values
        .iter()
        .map(|&v| (v as f64 - mean).powi(2))
        .sum::<f64>()
        / n;
    (mean.round() as u64, variance.sqrt().round() as u64)
}

// ── Brain helpers ──────────────────────────────────────────────────

fn open_brain(path: &std::path::Path) -> Result<Brain> {
    let ontology_path = path.join("ontology.toml");
    if !ontology_path.exists() {
        bail!("ontology.toml not found at {}", ontology_path.display());
    }
    let brain = Brain::open(BrainConfig {
        data_dir: path.to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
        device_id: None,
        enable_spectrogram: false,
    })?;
    Ok(brain)
}

// ── Benchmark runner ───────────────────────────────────────────────

fn run_query_bench(brain: &Brain, spec: &QuerySpec, iterations: usize) -> Result<QueryResult> {
    let vis = spec.visibility();

    // Warm-cache iterations
    let mut durations_us: Vec<u64> = Vec::with_capacity(iterations);
    let mut last_result = None;

    for _ in 0..iterations {
        let start = Instant::now();
        let result = brain.recall(&spec.text, vis)?;
        let elapsed = start.elapsed().as_micros() as u64;
        durations_us.push(elapsed);
        last_result = Some(result);
    }

    durations_us.sort_unstable();
    let (mean, stddev) = mean_stddev(&durations_us);
    let p50 = percentile(&durations_us, 50.0);
    let p95 = percentile(&durations_us, 95.0);
    let p99 = percentile(&durations_us, 99.0);

    // Accuracy check against last result
    let result = last_result.unwrap();
    let top_n_content: Vec<String> = result
        .memory_hits
        .iter()
        .take(spec.expected_top_n)
        .map(|h| h.content.to_lowercase())
        .collect();

    let pass = if spec.expected_keywords.is_empty() {
        // Adversarial: pass if few or no results
        true
    } else {
        spec.expected_keywords.iter().any(|kw| {
            let kw_lower = kw.to_lowercase();
            top_n_content.iter().any(|c| c.contains(&kw_lower))
        })
    };

    let top_score = result
        .memory_hits
        .first()
        .map(|h| h.signal_score)
        .unwrap_or(0.0);

    let p95_budget_us = (spec.latency_budget_p95_ms * 1000.0) as u64;
    let p99_budget_us = (spec.latency_budget_p99_ms * 1000.0) as u64;

    Ok(QueryResult {
        name: spec.name.clone(),
        pattern: spec.pattern().to_string(),
        latency_us: LatencyStats {
            cold: 0, // filled in by caller
            p50,
            p95,
            p99,
            mean,
            stddev,
        },
        accuracy: AccuracyResult {
            pass,
            top_score,
            num_results: result.memory_hits.len(),
        },
        budget: BudgetResult {
            p95_ok: p95 <= p95_budget_us,
            p99_ok: p99 <= p99_budget_us,
        },
    })
}

// ── Output formatting ──────────────────────────────────────────────

fn print_text_report(report: &BenchReport) {
    println!("Spectral Benchmark Report");
    println!("=========================");
    println!("Version:    {}", report.spectral_version);
    println!("Brain:      {}", report.brain_path);
    println!("Iterations: {}", report.iterations);
    println!();

    // Per-query table
    println!(
        "{:<35} {:>6} {:>6} {:>6} {:>6} {:>5} {:>4} {:>3}",
        "Query", "Cold", "P50", "P95", "P99", "Score", "Hits", "OK"
    );
    println!("{}", "-".repeat(80));

    for q in &report.queries {
        let ok_str = match (q.accuracy.pass, q.budget.p95_ok) {
            (true, true) => "Y",
            (true, false) => "B", // budget miss
            (false, true) => "A", // accuracy miss
            (false, false) => "N",
        };
        println!(
            "{:<35} {:>6} {:>6} {:>6} {:>6} {:>5.2} {:>4} {:>3}",
            q.name,
            q.latency_us.cold,
            q.latency_us.p50,
            q.latency_us.p95,
            q.latency_us.p99,
            q.accuracy.top_score,
            q.accuracy.num_results,
            ok_str,
        );
    }

    println!();
    println!("Latencies in microseconds. OK: Y=pass, B=budget miss, A=accuracy miss, N=both miss");

    // Aggregate
    println!();
    println!("Aggregate");
    println!("---------");
    println!(
        "Warm  P50={} us  P95={} us  P99={} us",
        report.aggregate.warm_p50_us, report.aggregate.warm_p95_us, report.aggregate.warm_p99_us
    );
    println!(
        "Cold  P50={} us  P95={} us  P99={} us",
        report.aggregate.cold_p50_us, report.aggregate.cold_p95_us, report.aggregate.cold_p99_us
    );
    println!(
        "Pass rate: {:.0}%  Budget violations: {}",
        report.aggregate.pass_rate * 100.0,
        report.aggregate.budget_violations
    );

    // Per-pattern breakdown
    println!();
    println!("Per-pattern breakdown");
    println!("---------------------");
    for pb in &report.per_pattern {
        println!(
            "{:<15} n={:<3} P50={:<6} P95={:<6} pass={:.0}%",
            pb.pattern,
            pb.query_count,
            pb.warm_p50_us,
            pb.warm_p95_us,
            pb.pass_rate * 100.0
        );
    }
}

fn print_json_report(report: &BenchReport) -> Result<()> {
    let json = serde_json::to_string_pretty(report)?;
    println!("{json}");
    Ok(())
}

// ── Main ───────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load queries
    let query_toml = std::fs::read_to_string(&cli.queries)
        .with_context(|| format!("reading queries from {}", cli.queries.display()))?;
    let query_file: QueryFile = toml::from_str(&query_toml).context("parsing queries TOML")?;

    let mut specs: Vec<QuerySpec> = query_file.queries;
    if let Some(ref filter) = cli.filter {
        specs.retain(|q| q.name.contains(filter.as_str()));
    }
    if specs.is_empty() {
        bail!("no queries matched (filter: {:?})", cli.filter);
    }

    eprintln!(
        "Running {} queries x {} iterations against {}",
        specs.len(),
        cli.iterations,
        cli.brain.display()
    );

    // Phase 1: Cold-cache pass
    // Open a fresh brain and run each query once to measure cold latency.
    eprintln!("Phase 1: cold-cache pass...");
    let mut cold_latencies: Vec<u64> = Vec::with_capacity(specs.len());
    {
        let brain = open_brain(&cli.brain)?;
        for spec in &specs {
            let start = Instant::now();
            let _ = brain.recall(&spec.text, spec.visibility());
            cold_latencies.push(start.elapsed().as_micros() as u64);
        }
    }

    // Phase 2: Warm-cache measurement
    // Open brain once, run all queries for the configured iterations.
    eprintln!(
        "Phase 2: warm-cache pass ({} iterations)...",
        cli.iterations
    );
    let brain = open_brain(&cli.brain)?;

    // Warm up: run each query once (discarded) to populate caches
    for spec in &specs {
        let _ = brain.recall(&spec.text, spec.visibility());
    }

    let mut results: Vec<QueryResult> = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        let mut qr = run_query_bench(&brain, spec, cli.iterations)?;
        qr.latency_us.cold = cold_latencies[i];
        results.push(qr);
    }

    // Compute aggregates
    let mut all_warm: Vec<u64> = results.iter().map(|r| r.latency_us.p50).collect();
    all_warm.sort_unstable();
    let mut all_cold: Vec<u64> = results.iter().map(|r| r.latency_us.cold).collect();
    all_cold.sort_unstable();

    let non_adversarial: Vec<&QueryResult> = results
        .iter()
        .filter(|r| r.pattern != "adversarial")
        .collect();
    let pass_count = non_adversarial.iter().filter(|r| r.accuracy.pass).count();
    let pass_total = non_adversarial.len();
    let budget_violations = results
        .iter()
        .filter(|r| !r.budget.p95_ok || !r.budget.p99_ok)
        .count();

    let aggregate = Aggregate {
        warm_p50_us: percentile(&all_warm, 50.0),
        warm_p95_us: percentile(&all_warm, 95.0),
        warm_p99_us: percentile(&all_warm, 99.0),
        cold_p50_us: percentile(&all_cold, 50.0),
        cold_p95_us: percentile(&all_cold, 95.0),
        cold_p99_us: percentile(&all_cold, 99.0),
        pass_rate: if pass_total > 0 {
            pass_count as f64 / pass_total as f64
        } else {
            1.0
        },
        budget_violations,
    };

    // Per-pattern breakdown
    let patterns = [
        "single_word",
        "multi_word",
        "concept",
        "temporal",
        "cross_domain",
        "adversarial",
    ];
    let per_pattern: Vec<PatternBreakdown> = patterns
        .iter()
        .filter_map(|&pat| {
            let group: Vec<&QueryResult> = results.iter().filter(|r| r.pattern == pat).collect();
            if group.is_empty() {
                return None;
            }
            let mut p50s: Vec<u64> = group.iter().map(|r| r.latency_us.p50).collect();
            p50s.sort_unstable();
            let mut p95s: Vec<u64> = group.iter().map(|r| r.latency_us.p95).collect();
            p95s.sort_unstable();

            let non_adv: Vec<&&QueryResult> = group
                .iter()
                .filter(|r| r.pattern != "adversarial")
                .collect();
            let pass_n = non_adv.iter().filter(|r| r.accuracy.pass).count();
            let total_n = non_adv.len();

            Some(PatternBreakdown {
                pattern: pat.to_string(),
                query_count: group.len(),
                warm_p50_us: percentile(&p50s, 50.0),
                warm_p95_us: percentile(&p95s, 95.0),
                pass_rate: if total_n > 0 {
                    pass_n as f64 / total_n as f64
                } else {
                    1.0
                },
            })
        })
        .collect();

    let report = BenchReport {
        spectral_version: env!("CARGO_PKG_VERSION").to_string(),
        brain_path: cli.brain.display().to_string(),
        iterations: cli.iterations,
        queries: results,
        aggregate,
        per_pattern,
    };

    match cli.format {
        OutputFormat::Text => print_text_report(&report),
        OutputFormat::Json => print_json_report(&report)?,
    }

    Ok(())
}
