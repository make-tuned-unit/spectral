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
    /// Parse the spec's visibility label.
    ///
    /// A typo used to fall through to `Private`, which silently changed what
    /// the benchmark measured — `Private` is the *admits-everything* context,
    /// so `visibility = "piblic"` quietly widened the query instead of
    /// failing. Unknown labels are now an error.
    fn visibility(&self) -> Result<Visibility> {
        match self.visibility.to_lowercase().as_str() {
            "private" => Ok(Visibility::Private),
            "team" => Ok(Visibility::Team),
            "org" => Ok(Visibility::Org),
            "public" => Ok(Visibility::Public),
            other => bail!(
                "query '{}': unknown visibility '{other}' \
                 (expected private, team, org, or public)",
                self.name
            ),
        }
    }

    /// Reject a spec that cannot express the check it claims to make.
    ///
    /// Each of these previously produced a *silent* wrong answer rather than
    /// an error: an unparseable visibility widened the query, and
    /// `expected_top_n = 0` made `take(0)` compare against nothing, so a
    /// keyword check could never pass and the query reported an accuracy
    /// failure that was really a spec bug.
    fn validate(&self) -> Result<()> {
        self.visibility()?;
        if self.name.trim().is_empty() {
            bail!("a query has an empty name");
        }
        if self.text.trim().is_empty() {
            bail!("query '{}': empty query text", self.name);
        }
        if self.expected_top_n == 0 {
            bail!(
                "query '{}': expected_top_n = 0 examines no results, so its \
                 accuracy check can never pass",
                self.name
            );
        }
        if self.latency_budget_p95_ms <= 0.0 || self.latency_budget_p99_ms <= 0.0 {
            bail!("query '{}': latency budgets must be positive", self.name);
        }
        if self.latency_budget_p99_ms < self.latency_budget_p95_ms {
            bail!(
                "query '{}': p99 budget ({}) is below its p95 budget ({})",
                self.name,
                self.latency_budget_p99_ms,
                self.latency_budget_p95_ms
            );
        }
        Ok(())
    }

    /// Is this an adversarial spec — one asserting that *few or no* results
    /// come back, rather than that a keyword appears?
    fn is_adversarial(&self) -> bool {
        self.expected_keywords.is_empty()
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

/// Nearest-rank percentile on a zero-based scale: index
/// `round(p/100 * (n-1))`, clamped. Input must already be sorted ascending.
///
/// Note this puts p50 of an even-sized sample on the upper of the two middle
/// values (p50 of 1..=100 is 51). Published bench figures use this
/// definition; see the test that pins it.
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

// ── Accuracy rule ──────────────────────────────────────────────────

/// Decide whether a query's results satisfy its spec.
///
/// Two rules, one per spec shape:
/// - **Keyword specs** pass when at least one expected keyword appears
///   (case-insensitive substring) in the content of any top-N result.
/// - **Adversarial specs** (no expected keywords) assert that the query
///   matches *few or no* results. They now actually check that: the query
///   passes only if it returned at most `expected_top_n` hits.
///
/// The adversarial branch previously returned `true` unconditionally, so an
/// adversarial query that matched the entire corpus still "passed" — the
/// check the spec file documents ("should return zero results") was never
/// performed. `expected_top_n` is reused as the threshold rather than
/// inventing a magic number, since that is the field the spec already uses
/// to say how many results it considers relevant.
fn evaluate_accuracy(spec: &QuerySpec, top_n_content: &[String], num_results: usize) -> bool {
    if spec.is_adversarial() {
        return num_results <= spec.expected_top_n;
    }
    spec.expected_keywords.iter().any(|kw| {
        let kw_lower = kw.to_lowercase();
        top_n_content.iter().any(|c| c.contains(&kw_lower))
    })
}

// ── Brain helpers ──────────────────────────────────────────────────

fn open_brain(path: &std::path::Path) -> Result<Brain> {
    let ontology_path = path.join("ontology.toml");
    if !ontology_path.exists() {
        bail!("ontology.toml not found at {}", ontology_path.display());
    }
    let mut config = BrainConfig {
        data_dir: path.to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
        device_id: None,
        enable_spectrogram: false,
        ..Default::default()
    };
    // Preserve the historical `SPECTRAL_*` env-var workflow for bench runs;
    // the library itself no longer reads env.
    spectral_bench_accuracy::apply_env_levers(&mut config);
    let brain = Brain::open(config)?;
    Ok(brain)
}

// ── Benchmark runner ───────────────────────────────────────────────

fn run_query_bench(brain: &Brain, spec: &QuerySpec, iterations: usize) -> Result<QueryResult> {
    // `iterations == 0` used to reach `last_result.unwrap()` below and panic
    // with no context. Refuse it up front instead.
    if iterations == 0 {
        bail!("iterations must be at least 1");
    }
    let vis = spec.visibility()?;

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

    // Accuracy check against last result. `iterations >= 1` is enforced at
    // entry, so this is populated.
    let result = last_result.expect("iterations >= 1 guarantees a result");
    let top_n_content: Vec<String> = result
        .memory_hits
        .iter()
        .take(spec.expected_top_n)
        .map(|h| h.content.to_lowercase())
        .collect();

    let pass = evaluate_accuracy(spec, &top_n_content, result.memory_hits.len());

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

    if cli.iterations == 0 {
        bail!("--iterations must be at least 1");
    }

    let mut specs: Vec<QuerySpec> = query_file.queries;
    if specs.is_empty() {
        bail!("{} defines no queries", cli.queries.display());
    }
    // Validate BEFORE opening a brain or running anything: a malformed spec
    // should fail in milliseconds, not after a cold pass over 30 queries.
    // Duplicate names silently produced two rows that could not be told apart
    // in the report.
    let mut seen = std::collections::HashSet::new();
    for spec in &specs {
        spec.validate()?;
        if !seen.insert(spec.name.as_str()) {
            bail!("duplicate query name '{}'", spec.name);
        }
    }
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
            let _ = brain.recall(&spec.text, spec.visibility()?);
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
        let _ = brain.recall(&spec.text, spec.visibility()?);
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

// ── Tests ──────────────────────────────────────────────────────────
//
// This crate had no tests at all. The logic worth pinning is the pure part:
// the statistics helpers, the accuracy rule, and spec validation. Everything
// else needs a populated brain on disk and belongs in the harness, not here.

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, keywords: &[&str], top_n: usize) -> QuerySpec {
        QuerySpec {
            name: name.into(),
            text: "some query text".into(),
            description: "d".into(),
            expected_keywords: keywords.iter().map(|s| s.to_string()).collect(),
            expected_top_n: top_n,
            latency_budget_p95_ms: 5.0,
            latency_budget_p99_ms: 10.0,
            visibility: "private".into(),
        }
    }

    // ── percentile ──

    #[test]
    fn percentile_is_empty_safe_and_clamped() {
        assert_eq!(percentile(&[], 50.0), 0, "empty input must not panic");
        assert_eq!(percentile(&[7], 99.0), 7, "single sample is every quantile");
        // Out-of-range p must clamp rather than index out of bounds.
        assert_eq!(percentile(&[1, 2, 3], 1000.0), 3);
        assert_eq!(percentile(&[1, 2, 3], 0.0), 1);
    }

    /// Pins the convention rather than asserting a preferred one: the index
    /// is `round(p/100 * (n-1))`, i.e. nearest-rank on a zero-based scale.
    /// For 1..=100 that puts p50 at 51, not 50, because the true median of an
    /// even-sized sample lies between two values and this rounds up.
    ///
    /// Deliberately NOT "corrected" — published bench numbers were produced
    /// with this definition, and changing it would silently move every
    /// historical latency figure. Documented instead.
    #[test]
    fn percentile_uses_the_p_times_n_minus_one_convention() {
        let s: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&s, 50.0), 51);
        assert_eq!(percentile(&s, 95.0), 95);
        assert_eq!(percentile(&s, 99.0), 99);
        assert_eq!(percentile(&s, 100.0), 100);
        // Odd-sized samples land exactly on the middle element.
        assert_eq!(percentile(&[1, 2, 3, 4, 5], 50.0), 3);
    }

    // ── mean_stddev ──

    #[test]
    fn mean_stddev_is_empty_safe() {
        assert_eq!(mean_stddev(&[]), (0, 0));
    }

    #[test]
    fn mean_stddev_matches_hand_computed_values() {
        // mean 4, population variance 8 -> stddev 2.83 -> rounds to 3
        assert_eq!(mean_stddev(&[2, 4, 4, 4, 5, 5, 7, 9]).0, 5);
        assert_eq!(mean_stddev(&[10, 10, 10]), (10, 0));
        let (m, sd) = mean_stddev(&[1, 3]);
        assert_eq!((m, sd), (2, 1));
    }

    // ── accuracy rule ──

    #[test]
    fn keyword_spec_passes_on_case_insensitive_substring() {
        let s = spec("concept_x", &["Runbook"], 5);
        let hits = vec!["the deploy runbook lives in notion".to_string()];
        assert!(evaluate_accuracy(&s, &hits, 1));
    }

    #[test]
    fn keyword_spec_fails_when_no_keyword_appears() {
        let s = spec("concept_x", &["runbook"], 5);
        let hits = vec!["entirely unrelated content".to_string()];
        assert!(!evaluate_accuracy(&s, &hits, 1));
    }

    /// The regression this rule exists for: an adversarial spec asserts
    /// "few or no results", and the old code returned `true` unconditionally,
    /// so a nonsense query matching the whole corpus still passed.
    #[test]
    fn adversarial_spec_fails_when_it_matches_too_much() {
        let s = spec("adversarial_gibberish", &[], 5);
        assert!(
            evaluate_accuracy(&s, &[], 0),
            "zero results is the ideal adversarial outcome"
        );
        assert!(
            evaluate_accuracy(&s, &[], 5),
            "at the threshold, still a pass"
        );
        assert!(
            !evaluate_accuracy(&s, &[], 6),
            "an adversarial query matching more than expected_top_n must FAIL; \
             returning true unconditionally made this check vacuous"
        );
        assert!(!evaluate_accuracy(&s, &[], 500));
    }

    // ── spec validation ──

    #[test]
    fn unknown_visibility_is_rejected_not_silently_private() {
        let mut s = spec("q", &["k"], 5);
        s.visibility = "piblic".into();
        let err = s.validate().unwrap_err().to_string();
        assert!(err.contains("unknown visibility"), "got: {err}");
    }

    #[test]
    fn every_known_visibility_label_parses() {
        for (label, want) in [
            ("private", Visibility::Private),
            ("TEAM", Visibility::Team),
            ("Org", Visibility::Org),
            ("public", Visibility::Public),
        ] {
            let mut s = spec("q", &["k"], 5);
            s.visibility = label.into();
            assert_eq!(s.visibility().unwrap(), want, "label {label}");
        }
    }

    #[test]
    fn zero_expected_top_n_is_rejected() {
        // take(0) compares against nothing, so the check could never pass —
        // it reported an accuracy failure that was really a spec bug.
        let s = spec("q", &["k"], 0);
        assert!(s
            .validate()
            .unwrap_err()
            .to_string()
            .contains("expected_top_n"));
    }

    #[test]
    fn empty_name_or_text_is_rejected() {
        let mut s = spec("", &["k"], 5);
        assert!(s.validate().is_err());
        s = spec("q", &["k"], 5);
        s.text = "   ".into();
        assert!(s
            .validate()
            .unwrap_err()
            .to_string()
            .contains("empty query text"));
    }

    #[test]
    fn inverted_or_nonpositive_latency_budgets_are_rejected() {
        let mut s = spec("q", &["k"], 5);
        s.latency_budget_p99_ms = 1.0; // below p95
        assert!(s
            .validate()
            .unwrap_err()
            .to_string()
            .contains("below its p95"));

        s = spec("q", &["k"], 5);
        s.latency_budget_p95_ms = 0.0;
        assert!(s.validate().unwrap_err().to_string().contains("positive"));
    }

    #[test]
    fn a_wellformed_spec_validates() {
        assert!(spec("single_word_agent", &["agent"], 5).validate().is_ok());
        assert!(spec("adversarial_gibberish", &[], 5).validate().is_ok());
    }

    // ── pattern classification ──

    #[test]
    fn pattern_classifies_by_name_prefix_and_defaults_to_other() {
        for (name, want) in [
            ("single_word_agent", "single_word"),
            ("multi_word_thing", "multi_word"),
            ("concept_x", "concept"),
            ("temporal_y", "temporal"),
            ("cross_domain_z", "cross_domain"),
            ("adversarial_gibberish", "adversarial"),
            ("something_else", "other"),
        ] {
            assert_eq!(spec(name, &["k"], 5).pattern(), want, "name {name}");
        }
    }

    // ── the shipped query file ──

    /// The committed `queries.toml` must satisfy the validator it is run
    /// through, so a malformed spec is caught here rather than after a cold
    /// pass over 30 queries against a real brain.
    #[test]
    fn shipped_queries_toml_is_valid() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/queries.toml");
        let toml_src = std::fs::read_to_string(path).expect("queries.toml is committed");
        let file: QueryFile = toml::from_str(&toml_src).expect("queries.toml parses");
        assert!(!file.queries.is_empty());
        let mut seen = std::collections::HashSet::new();
        for spec in &file.queries {
            spec.validate()
                .unwrap_or_else(|e| panic!("shipped spec '{}' is invalid: {e}", spec.name));
            assert!(
                seen.insert(spec.name.as_str()),
                "duplicate query name '{}' in queries.toml",
                spec.name
            );
        }
    }

    // ── output format parsing ──

    #[test]
    fn output_format_parses_case_insensitively_and_rejects_junk() {
        assert!(matches!(
            "TEXT".parse::<OutputFormat>(),
            Ok(OutputFormat::Text)
        ));
        assert!(matches!(
            "json".parse::<OutputFormat>(),
            Ok(OutputFormat::Json)
        ));
        assert!("yaml".parse::<OutputFormat>().is_err());
    }
}
