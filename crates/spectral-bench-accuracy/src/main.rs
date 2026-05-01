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
    },

    /// Pretty-print a previously saved JSON report
    Report {
        /// Path to the JSON report file
        #[arg(long)]
        path: PathBuf,
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
        } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;
            let question_count = max_questions.unwrap_or(ds.questions.len());
            let estimated_cost = eval::estimate_cost(question_count);

            eprintln!("Dataset: {} questions", ds.questions.len());
            eprintln!("Evaluating: {question_count} questions");
            eprintln!("Estimated cost: ${estimated_cost:.2}");

            if estimated_cost > 10.0 && !confirm_cost {
                eprintln!("Cost exceeds $10. Pass --confirm-cost to proceed.");
                std::process::exit(1);
            }

            let cats = categories.map(|s| {
                s.split(',')
                    .map(|c| Category::from_question_type(c.trim()))
                    .collect()
            });

            let strategy = match ingest_strategy.as_str() {
                "per_session" => ingest::IngestStrategy::PerSession,
                _ => ingest::IngestStrategy::PerTurn,
            };

            let config = EvalConfig {
                dataset_path: dataset,
                work_dir,
                max_questions,
                categories: cats,
                ingest_strategy: strategy,
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

        Command::DryRun { dataset, work_dir } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;
            let question = ds
                .questions
                .first()
                .ok_or_else(|| anyhow::anyhow!("empty dataset"))?;

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
