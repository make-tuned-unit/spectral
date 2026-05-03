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

        /// Retrieval path: tact (default) or graph
        #[arg(long, default_value = "tact")]
        retrieval_path: String,

        /// Write per-memory signal score records to this JSONL path
        #[arg(long)]
        dump_scores: Option<PathBuf>,

        /// Use cascade retrieval (L1→L2→L3) instead of direct recall
        #[arg(long)]
        use_cascade: bool,
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
        } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;
            let question_count = max_questions.unwrap_or(ds.len());
            let estimated_cost = eval::estimate_cost(question_count);

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
                "graph" => retrieval::RetrievalPath::Graph,
                _ => retrieval::RetrievalPath::Tact,
            };

            let config = EvalConfig {
                dataset_path: dataset,
                work_dir,
                max_questions,
                categories: cats,
                ingest_strategy: strategy,
                retrieval_path: ret_path,
                use_cascade,
                dump_scores_path: dump_scores,
                ..Default::default()
            };

            let actor = AnthropicActor::from_env()?;
            let judge = AnthropicJudge::from_env()?;

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
    }

    Ok(())
}
