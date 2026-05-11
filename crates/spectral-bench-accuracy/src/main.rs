use anyhow::Result;
use clap::{Parser, Subcommand};
use spectral_bench_accuracy::*;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "spectral-bench-accuracy",
    about = "Accuracy benchmarks for Spectral agent memory (LongMemEval_S)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the full evaluation
    Run {
        /// Path to the LongMemEval_S dataset JSON
        #[arg(long)]
        dataset: PathBuf,

        /// Working directory for brain instances and checkpoints
        #[arg(long, default_value = "eval-work")]
        work_dir: PathBuf,

        /// Maximum questions to evaluate (default: all)
        #[arg(long)]
        max_questions: Option<usize>,

        /// Categories to include (comma-separated)
        #[arg(long)]
        categories: Option<String>,

        /// Output file for the JSON report
        #[arg(long, default_value = "eval-report.json")]
        output: PathBuf,

        /// Confirm estimated cost (required if > $10)
        #[arg(long)]
        confirm_cost: bool,

        /// Ingestion strategy: per_turn or per_session
        #[arg(long, default_value = "per_turn")]
        ingest_strategy: String,

        /// Retrieval path: tact, graph, topk_fts (default), or cascade
        #[arg(long, default_value = "topk_fts")]
        retrieval_path: String,

        /// Write per-memory signal score records to this JSONL path
        #[arg(long)]
        dump_scores: Option<PathBuf>,

        /// Use cascade retrieval (L1→L2→L3) instead of direct recall
        #[arg(long)]
        use_cascade: bool,

        /// Actor model name
        #[arg(long, default_value = "claude-sonnet-4-6")]
        actor_model: String,

        /// Judge model name
        #[arg(long, default_value = "claude-sonnet-4-6")]
        judge_model: String,

        /// Base URL for API calls
        #[arg(long, default_value = "https://api.anthropic.com")]
        base_url: String,

        /// Maximum memories to pass to actor
        #[arg(long, default_value = "40")]
        max_results: usize,
    },

    /// Pretty-print a previously saved JSON report
    Report {
        /// Path to the JSON report file
        #[arg(long)]
        path: PathBuf,
    },

    /// Deep-inspect a single question: ingest, recall, enumerate all memories
    Inspect {
        /// Path to the LongMemEval_S dataset JSON
        #[arg(long)]
        dataset: PathBuf,

        /// Question ID to inspect
        #[arg(long)]
        question_id: String,

        /// Working directory
        #[arg(long, default_value = "eval-work")]
        work_dir: PathBuf,

        /// Output JSON file
        #[arg(long, default_value = "inspect.json")]
        output: PathBuf,
    },

    /// Dry-run: ingest one question, retrieve, but don't call LLMs
    DryRun {
        /// Path to the LongMemEval_S dataset JSON
        #[arg(long)]
        dataset: PathBuf,

        /// Working directory
        #[arg(long, default_value = "eval-work")]
        work_dir: PathBuf,
    },

    /// Pre-flight: ingest questions and run spot-check SQL queries.
    /// Verifies that time_delta_bucket, wings, halls, and spectrograms are populated.
    Preflight {
        /// Path to the LongMemEval_S dataset JSON
        #[arg(long)]
        dataset: PathBuf,

        /// Working directory
        #[arg(long, default_value = "eval-work")]
        work_dir: PathBuf,

        /// Question ID to inspect (default: first multi-session question)
        #[arg(long)]
        question_id: Option<String>,

        /// Check ALL questions in the dataset (overrides --question-id)
        #[arg(long)]
        all: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Run {
            dataset,
            work_dir,
            max_questions,
            categories,
            output,
            confirm_cost,
            ingest_strategy,
            retrieval_path,
            dump_scores,
            use_cascade,
            actor_model,
            judge_model,
            base_url,
            max_results,
        } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;
            let question_count = max_questions.unwrap_or(ds.len());
            let estimated_cost =
                eval::estimate_cost_for_models(question_count, &actor_model, &judge_model);

            eprintln!("Dataset: {} questions", ds.len());
            eprintln!("Evaluating: {question_count} questions");
            eprintln!("Estimated cost: ${estimated_cost:.2}");

            if estimated_cost > 10.0 && !confirm_cost {
                eprintln!("Cost exceeds $10. Pass --confirm-cost to proceed.");
                std::process::exit(1);
            }

            let cats = categories
                .map(|s| {
                    s.split(',')
                        .map(|c| Category::from_question_type(c.trim()))
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?;

            let strategy = match ingest_strategy.as_str() {
                "per_session" => ingest::IngestStrategy::PerSession,
                _ => ingest::IngestStrategy::PerTurn,
            };

            let ret_path = match retrieval_path.as_str() {
                "tact" => retrieval::RetrievalPath::Tact,
                "graph" => retrieval::RetrievalPath::Graph,
                "topk_fts" => retrieval::RetrievalPath::TopkFts,
                "cascade" => retrieval::RetrievalPath::Cascade,
                other => {
                    eprintln!(
                        "Unknown retrieval path: {other}. Valid: tact, graph, topk_fts, cascade"
                    );
                    std::process::exit(1);
                }
            };
            let ret_path = if use_cascade {
                retrieval::RetrievalPath::Cascade
            } else {
                ret_path
            };

            let config = EvalConfig {
                dataset_path: dataset,
                work_dir,
                max_questions,
                categories: cats,
                ingest_strategy: strategy,
                retrieval: RetrievalConfig { max_results },
                retrieval_path: ret_path,
                use_cascade,
                dump_scores_path: dump_scores,
                ..Default::default()
            };

            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
            let actor = AnthropicActor::new(api_key.clone(), actor_model, base_url.clone());
            let judge = AnthropicJudge::new(api_key, judge_model, base_url);

            let eval = AccuracyEval::new(config, Box::new(actor), Box::new(judge));
            let report = eval.run()?;

            println!("{}", report.summary());
            report::save_report(&report, &output)?;
            eprintln!("\nReport saved to {}", output.display());
        }

        Command::Report { path } => {
            let report = report::load_report(&path)?;
            println!("{}", report.summary());
        }

        Command::Inspect {
            dataset,
            question_id,
            work_dir,
            output,
        } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;
            let question = ds
                .iter()
                .find(|q| q.question_id == question_id)
                .ok_or_else(|| anyhow::anyhow!("question_id {question_id} not found in dataset"))?;

            std::fs::create_dir_all(&work_dir)?;
            eprintln!("Inspecting question {question_id}...");
            let result = spectral_bench_accuracy::inspect::inspect_question(
                question,
                &work_dir,
                &RetrievalConfig::default(),
            )?;

            let json = serde_json::to_string_pretty(&result)?;
            std::fs::write(&output, &json)?;

            eprintln!(
                "Total memories in haystack: {}",
                result.haystack_memory_count
            );
            eprintln!("Top-20 retrieved: {}", result.retrieved_top_20.len());
            eprintln!("All memories enumerated: {}", result.all_memories.len());
            eprintln!("\nInspect output saved to {}", output.display());
        }

        Command::DryRun { dataset, work_dir } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;
            let question = ds.first().ok_or_else(|| anyhow::anyhow!("empty dataset"))?;

            eprintln!("Dry-run: ingesting question {}", question.question_id);
            let brain_dir = work_dir.join(format!("dryrun_{}", question.question_id));
            let brain =
                ingest::ingest_question(question, &brain_dir, ingest::IngestStrategy::PerTurn)?;

            let memories =
                retrieval::retrieve(&brain, &question.question, &RetrievalConfig::default())?;
            eprintln!(
                "Retrieved {} memories for: {}",
                memories.len(),
                question.question
            );
            for (i, m) in memories.iter().enumerate().take(5) {
                eprintln!("  {}: {}", i + 1, &m[..m.len().min(120)]);
            }
            eprintln!("\nDry-run complete. No LLM calls made.");
            let _ = std::fs::remove_dir_all(&brain_dir);
        }

        Command::Preflight {
            dataset,
            work_dir,
            question_id,
            all,
        } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;
            std::fs::create_dir_all(&work_dir)?;

            let questions: Vec<&spectral_bench_accuracy::dataset::Question> = if all {
                ds.iter().collect()
            } else {
                let q = match &question_id {
                    Some(id) => ds
                        .iter()
                        .find(|q| q.question_id == *id)
                        .ok_or_else(|| anyhow::anyhow!("question_id {id} not found"))?,
                    None => ds
                        .iter()
                        .find(|q| {
                            q.question_type == "multi-session" && q.haystack_sessions.len() >= 3
                        })
                        .or_else(|| ds.first())
                        .ok_or_else(|| anyhow::anyhow!("empty dataset"))?,
                };
                vec![q]
            };

            eprintln!(
                "Preflight: checking {} question(s) from dataset ({} total)",
                questions.len(),
                ds.len()
            );

            // Accumulators across all questions
            let mut total_memories: i64 = 0;
            let mut total_null_wings: i64 = 0;
            let mut total_null_halls: i64 = 0;
            let mut total_spectrograms: i64 = 0;
            let mut total_fingerprints: i64 = 0;
            let mut total_null_buckets: i64 = 0;
            let mut total_unknown_buckets: i64 = 0;
            let mut wing_counts: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            let mut hall_counts: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            let mut bucket_counts: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            let mut failed_questions: Vec<String> = Vec::new();

            let pb = indicatif::ProgressBar::new(questions.len() as u64);
            pb.set_style(
                indicatif::ProgressStyle::default_bar()
                    .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                    .unwrap()
                    .progress_chars("#>-"),
            );

            for question in &questions {
                let brain_dir = work_dir.join(format!("preflight_{}", question.question_id));
                let _ = std::fs::remove_dir_all(&brain_dir);

                match ingest::ingest_question(question, &brain_dir, ingest::IngestStrategy::PerTurn)
                {
                    Ok(_brain) => {}
                    Err(e) => {
                        failed_questions
                            .push(format!("{}: ingest error: {e}", question.question_id));
                        pb.inc(1);
                        let _ = std::fs::remove_dir_all(&brain_dir);
                        continue;
                    }
                }

                let db_path = brain_dir.join("memory.db");
                let conn = match rusqlite::Connection::open_with_flags(
                    &db_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        failed_questions
                            .push(format!("{}: db open error: {e}", question.question_id));
                        pb.inc(1);
                        let _ = std::fs::remove_dir_all(&brain_dir);
                        continue;
                    }
                };

                // Memory count
                let mem_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))?;
                total_memories += mem_count;

                // Null wings/halls
                let nw: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE wing IS NULL",
                    [],
                    |r| r.get(0),
                )?;
                let nh: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE hall IS NULL",
                    [],
                    |r| r.get(0),
                )?;
                total_null_wings += nw;
                total_null_halls += nh;

                // Wing distribution
                let mut stmt = conn
                    .prepare("SELECT COALESCE(wing, 'NULL'), COUNT(*) FROM memories GROUP BY 1")?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                for row in rows {
                    let (wing, count) = row?;
                    *wing_counts.entry(wing).or_insert(0) += count;
                }

                // Hall distribution
                let mut stmt = conn
                    .prepare("SELECT COALESCE(hall, 'NULL'), COUNT(*) FROM memories GROUP BY 1")?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                for row in rows {
                    let (hall, count) = row?;
                    *hall_counts.entry(hall).or_insert(0) += count;
                }

                // Spectrograms
                let has_spec: bool = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_spectrogram'",
                        [],
                        |r| r.get::<_, i64>(0),
                    )
                    .map(|c| c > 0)?;
                if has_spec {
                    let sc: i64 =
                        conn.query_row("SELECT COUNT(*) FROM memory_spectrogram", [], |r| {
                            r.get(0)
                        })?;
                    total_spectrograms += sc;
                }

                // Fingerprints
                let has_fp: bool = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='constellation_fingerprints'",
                        [],
                        |r| r.get::<_, i64>(0),
                    )
                    .map(|c| c > 0)?;
                if has_fp {
                    let fp: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM constellation_fingerprints",
                        [],
                        |r| r.get(0),
                    )?;
                    total_fingerprints += fp;

                    let nb: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM constellation_fingerprints WHERE time_delta_bucket IS NULL",
                        [],
                        |r| r.get(0),
                    )?;
                    let ub: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM constellation_fingerprints WHERE time_delta_bucket = 'unknown'",
                        [],
                        |r| r.get(0),
                    )?;
                    total_null_buckets += nb;
                    total_unknown_buckets += ub;

                    let mut stmt = conn.prepare(
                        "SELECT COALESCE(time_delta_bucket, 'NULL'), COUNT(*) FROM constellation_fingerprints GROUP BY 1",
                    )?;
                    let rows =
                        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                    for row in rows {
                        let (bucket, count) = row?;
                        *bucket_counts.entry(bucket).or_insert(0) += count;
                    }
                }

                let _ = std::fs::remove_dir_all(&brain_dir);
                pb.inc(1);
            }

            pb.finish_and_clear();

            // ── Report ──
            eprintln!(
                "\n=== AGGREGATE SPOT-CHECK ({} questions) ===",
                questions.len()
            );
            eprintln!("Total memories: {total_memories}");

            eprintln!("\nWing distribution:");
            let mut wings: Vec<_> = wing_counts.iter().collect();
            wings.sort_by(|a, b| b.1.cmp(a.1));
            for (wing, count) in &wings {
                eprintln!("  {wing}: {count}");
            }

            eprintln!("\nHall distribution:");
            let mut halls: Vec<_> = hall_counts.iter().collect();
            halls.sort_by(|a, b| b.1.cmp(a.1));
            for (hall, count) in &halls {
                eprintln!("  {hall}: {count}");
            }

            eprintln!("\nSpectrograms: {total_spectrograms} / {total_memories}");

            eprintln!("\nFingerprints: {total_fingerprints}");
            if !bucket_counts.is_empty() {
                eprintln!("time_delta_bucket distribution:");
                let mut buckets: Vec<_> = bucket_counts.iter().collect();
                buckets.sort_by(|a, b| b.1.cmp(a.1));
                for (bucket, count) in &buckets {
                    eprintln!("  {bucket}: {count}");
                }
            }

            // ── Summary ──
            eprintln!("\n=== PREFLIGHT SUMMARY ===");
            let mut pass = true;

            if total_null_wings > 0 {
                eprintln!("FAIL: {total_null_wings} memories with NULL wing");
                pass = false;
            } else {
                eprintln!("PASS: All {total_memories} memories have wing assigned");
            }

            if total_null_halls > 0 {
                eprintln!("FAIL: {total_null_halls} memories with NULL hall");
                pass = false;
            } else {
                eprintln!("PASS: All {total_memories} memories have hall assigned");
            }

            if total_spectrograms == total_memories {
                eprintln!(
                    "PASS: All memories have spectrograms ({total_spectrograms}/{total_memories})"
                );
            } else {
                eprintln!("FAIL: Spectrogram coverage {total_spectrograms}/{total_memories}");
                pass = false;
            }

            if total_null_buckets > 0 {
                eprintln!("FAIL: {total_null_buckets} fingerprints with NULL time_delta_bucket");
                pass = false;
            }
            if total_unknown_buckets > 0 {
                eprintln!(
                    "FAIL: {total_unknown_buckets} fingerprints with 'unknown' time_delta_bucket"
                );
                pass = false;
            }
            if total_null_buckets == 0 && total_unknown_buckets == 0 {
                eprintln!(
                    "PASS: All {total_fingerprints} fingerprints have valid time_delta_bucket"
                );
            }

            if !failed_questions.is_empty() {
                eprintln!("\nFailed questions ({}):", failed_questions.len());
                for f in &failed_questions {
                    eprintln!("  {f}");
                }
                pass = false;
            }

            if pass {
                eprintln!("\nAll pre-flight checks passed. Ready for bench run.");
            } else {
                eprintln!("\nPre-flight checks FAILED. Fix issues before bench run.");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
