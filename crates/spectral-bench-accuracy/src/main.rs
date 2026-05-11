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

    /// Pre-flight: ingest one question, run spot-check SQL queries on the brain.
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
        } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;
            let question = match &question_id {
                Some(id) => ds
                    .iter()
                    .find(|q| q.question_id == *id)
                    .ok_or_else(|| anyhow::anyhow!("question_id {id} not found"))?,
                None => ds
                    .iter()
                    .find(|q| q.question_type == "multi-session" && q.haystack_sessions.len() >= 3)
                    .or_else(|| ds.first())
                    .ok_or_else(|| anyhow::anyhow!("empty dataset"))?,
            };

            eprintln!(
                "Preflight: ingesting question {} ({} sessions, {} turns total)",
                question.question_id,
                question.haystack_sessions.len(),
                question
                    .haystack_sessions
                    .iter()
                    .map(|s| s.len())
                    .sum::<usize>()
            );

            std::fs::create_dir_all(&work_dir)?;
            let brain_dir = work_dir.join(format!("preflight_{}", question.question_id));
            let _ = std::fs::remove_dir_all(&brain_dir);
            let _brain =
                ingest::ingest_question(question, &brain_dir, ingest::IngestStrategy::PerTurn)?;

            // Open the raw SQLite to run spot-check queries
            let db_path = brain_dir.join("memory.db");
            let conn = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;

            eprintln!("\n=== SPOT-CHECK: memories ===");

            // 1. Total memory count
            let total: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))?;
            eprintln!("Total memories: {total}");

            // 2. Wing distribution
            eprintln!("\nWing distribution:");
            let mut stmt = conn.prepare(
                "SELECT COALESCE(wing, 'NULL'), COUNT(*) FROM memories GROUP BY 1 ORDER BY 2 DESC",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            let mut null_wings = 0i64;
            for row in rows {
                let (wing, count) = row?;
                if wing == "NULL" {
                    null_wings = count;
                }
                eprintln!("  {wing}: {count}");
            }

            // 3. Hall distribution
            eprintln!("\nHall distribution:");
            let mut stmt = conn.prepare(
                "SELECT COALESCE(hall, 'NULL'), COUNT(*) FROM memories GROUP BY 1 ORDER BY 2 DESC",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            let mut null_halls = 0i64;
            for row in rows {
                let (hall, count) = row?;
                if hall == "NULL" {
                    null_halls = count;
                }
                eprintln!("  {hall}: {count}");
            }

            // 4. Spectrogram coverage
            eprintln!("\n=== SPOT-CHECK: spectrograms ===");
            let has_table: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_spectrogram'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)?;

            if has_table {
                let spectrogram_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM memory_spectrogram", [], |r| r.get(0))?;
                eprintln!("Memories with spectrograms: {spectrogram_count} / {total}");
                if spectrogram_count < total {
                    eprintln!(
                        "  WARNING: {} memories missing spectrograms",
                        total - spectrogram_count
                    );
                }
            } else {
                eprintln!("  WARNING: memory_spectrogram table does not exist");
            }

            // 5. Fingerprint time_delta_bucket distribution
            eprintln!("\n=== SPOT-CHECK: constellation fingerprints ===");
            let has_fp_table: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='constellation_fingerprints'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)?;

            if has_fp_table {
                let fp_total: i64 =
                    conn.query_row("SELECT COUNT(*) FROM constellation_fingerprints", [], |r| {
                        r.get(0)
                    })?;
                eprintln!("Total fingerprints: {fp_total}");

                let null_buckets: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM constellation_fingerprints WHERE time_delta_bucket IS NULL",
                    [],
                    |r| r.get(0),
                )?;
                let unknown_buckets: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM constellation_fingerprints WHERE time_delta_bucket = 'unknown'",
                    [],
                    |r| r.get(0),
                )?;

                eprintln!("\ntime_delta_bucket distribution:");
                let mut stmt = conn.prepare(
                    "SELECT COALESCE(time_delta_bucket, 'NULL'), COUNT(*) FROM constellation_fingerprints GROUP BY 1 ORDER BY 2 DESC",
                )?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                for row in rows {
                    let (bucket, count) = row?;
                    eprintln!("  {bucket}: {count}");
                }

                if null_buckets > 0 {
                    eprintln!("\n  FAIL: {null_buckets} fingerprints with NULL time_delta_bucket");
                }
                if unknown_buckets > 0 {
                    eprintln!(
                        "  FAIL: {unknown_buckets} fingerprints with 'unknown' time_delta_bucket"
                    );
                }
                if null_buckets == 0 && unknown_buckets == 0 && fp_total > 0 {
                    eprintln!("\n  PASS: All fingerprints have valid time_delta_bucket");
                }
            } else {
                eprintln!("  No constellation_fingerprints table (no fingerprints generated)");
                eprintln!("  This is expected if all memories land in wing='general'");
            }

            // Summary
            eprintln!("\n=== PREFLIGHT SUMMARY ===");
            let mut pass = true;
            if null_wings > 0 {
                eprintln!("FAIL: {null_wings} memories with NULL wing");
                pass = false;
            } else {
                eprintln!("PASS: All memories have wing assigned");
            }
            if null_halls > 0 {
                eprintln!("FAIL: {null_halls} memories with NULL hall");
                pass = false;
            } else {
                eprintln!("PASS: All memories have hall assigned");
            }

            if has_table {
                let spectrogram_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM memory_spectrogram", [], |r| r.get(0))?;
                if spectrogram_count == total {
                    eprintln!("PASS: All memories have spectrograms ({spectrogram_count}/{total})");
                } else {
                    eprintln!("FAIL: Spectrogram coverage {spectrogram_count}/{total}");
                    pass = false;
                }
            } else {
                eprintln!("FAIL: No spectrogram table");
                pass = false;
            }

            if pass {
                eprintln!("\nAll pre-flight checks passed. Ready for bench run.");
            } else {
                eprintln!("\nPre-flight checks FAILED. Fix issues before bench run.");
            }

            // Clean up
            let _ = std::fs::remove_dir_all(&brain_dir);
        }
    }

    Ok(())
}
