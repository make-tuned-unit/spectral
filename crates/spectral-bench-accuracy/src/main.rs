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

        /// Retrieval path: tact, graph, topk_fts, or cascade.
        /// When explicitly set, overrides per-question shape routing.
        /// When omitted with --use-cascade, shape routing is active.
        #[arg(long)]
        retrieval_path: Option<String>,

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

        /// Path to descriptions JSON file (from `describe` subcommand).
        /// When provided, descriptions are applied to each question's brain
        /// after ingestion, enriching FTS indexing.
        #[arg(long)]
        descriptions: Option<PathBuf>,

        /// Filter to a single question by ID (for targeted pre-validation).
        #[arg(long)]
        question_id: Option<String>,
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

        /// Path to descriptions JSON file (from `describe` subcommand).
        /// When provided, descriptions are applied after ingestion to
        /// enrich FTS indexing before retrieval.
        #[arg(long)]
        descriptions: Option<PathBuf>,

        /// Retrieval path: cascade (default) or local.
        /// cascade matches bench --use-cascade behavior.
        /// local uses legacy recall_local for debugging.
        #[arg(long, default_value = "cascade")]
        retrieval_path: String,
    },

    /// Generate search-indexing descriptions for bench memories via LLM API
    Describe {
        /// Path to the LongMemEval_S dataset JSON
        #[arg(long)]
        dataset: PathBuf,

        /// Output JSON file for descriptions
        #[arg(long, default_value = "bench_descriptions.json")]
        output: PathBuf,

        /// Force regeneration of all descriptions (default: skip existing)
        #[arg(long)]
        regenerate: bool,

        /// Ingestion strategy: per_turn or per_session
        #[arg(long, default_value = "per_turn")]
        ingest_strategy: String,

        /// Model for description generation
        #[arg(long, default_value = "claude-haiku-4-5-20251001")]
        model: String,

        /// Base URL for API calls
        #[arg(long, default_value = "https://api.anthropic.com")]
        base_url: String,

        /// Maximum questions to process (default: all)
        #[arg(long)]
        max_questions: Option<usize>,

        /// Categories to include (comma-separated)
        #[arg(long)]
        categories: Option<String>,
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
            actor_model,
            judge_model,
            base_url,
            max_results,
            descriptions,
            question_id,
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

            // Parse explicit retrieval path if provided.
            let explicit_path = retrieval_path.as_ref().map(|rp| match rp.as_str() {
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
            });

            // Precedence for retrieval routing:
            // 1. Explicit --retrieval-path X → all questions use X (shape routing disabled).
            // 2. --use-cascade without explicit path → shape routing active.
            // 3. Neither → default (topk_fts).
            let (ret_path, retrieval_path_override) = match (explicit_path, use_cascade) {
                (Some(path), _) => (path, Some(path)),
                (None, true) => (retrieval::RetrievalPath::Cascade, None),
                (None, false) => (
                    retrieval::RetrievalPath::TopkFts,
                    Some(retrieval::RetrievalPath::TopkFts),
                ),
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
                retrieval_path_override,
                question_id,
                ..Default::default()
            };

            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
            let actor = AnthropicActor::new(api_key.clone(), actor_model, base_url.clone());
            let judge = AnthropicJudge::new(api_key, judge_model, base_url);

            let mut eval = AccuracyEval::new(config, Box::new(actor), Box::new(judge));
            if let Some(ref desc_path) = descriptions {
                let descs = spectral_bench_accuracy::describe::load_descriptions(desc_path)?;
                eprintln!("Loaded {} descriptions from {}", descs.len(), desc_path.display());
                eval = eval.with_descriptions(descs);
            }
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
            descriptions,
            retrieval_path,
        } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;
            let question = ds
                .iter()
                .find(|q| q.question_id == question_id)
                .ok_or_else(|| anyhow::anyhow!("question_id {question_id} not found in dataset"))?;

            let descs = descriptions
                .as_ref()
                .map(|p| {
                    let d = spectral_bench_accuracy::describe::load_descriptions(p)?;
                    eprintln!("Loaded {} descriptions from {}", d.len(), p.display());
                    Ok::<_, anyhow::Error>(d)
                })
                .transpose()?;

            let ret_path = match retrieval_path.as_str() {
                "local" => spectral_bench_accuracy::inspect::InspectRetrievalPath::Local,
                "cascade" => spectral_bench_accuracy::inspect::InspectRetrievalPath::Cascade,
                other => {
                    eprintln!("Unknown retrieval path: {other}. Valid: cascade, local");
                    std::process::exit(1);
                }
            };

            std::fs::create_dir_all(&work_dir)?;
            eprintln!(
                "Inspecting question {question_id} (retrieval: {retrieval_path})..."
            );
            let result = spectral_bench_accuracy::inspect::inspect_question(
                question,
                &work_dir,
                &RetrievalConfig::default(),
                descs.as_ref(),
                ret_path,
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

        Command::Describe {
            dataset,
            output,
            regenerate,
            ingest_strategy,
            model,
            base_url,
            max_questions,
            categories,
        } => {
            let ds = spectral_bench_accuracy::dataset::load_dataset(&dataset)?;

            let cats = categories
                .map(|s| {
                    s.split(',')
                        .map(|c| Category::from_question_type(c.trim()))
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?;

            let mut questions: Vec<&spectral_bench_accuracy::dataset::Question> =
                ds.iter().collect();
            if let Some(ref cat_filter) = cats {
                let cat_strs: std::collections::HashSet<String> =
                    cat_filter.iter().map(|c| c.as_str().to_string()).collect();
                questions.retain(|q| {
                    Category::from_question_type(&q.question_type)
                        .map(|cat| cat_strs.contains(cat.as_str()))
                        .unwrap_or(false)
                });
            }
            if let Some(max) = max_questions {
                questions.truncate(max);
            }

            let strategy = match ingest_strategy.as_str() {
                "per_session" => ingest::IngestStrategy::PerSession,
                _ => ingest::IngestStrategy::PerTurn,
            };

            // Load existing descriptions for idempotence
            let existing = spectral_bench_accuracy::describe::load_descriptions(&output)?;
            eprintln!(
                "Loaded {} existing descriptions from {}",
                existing.len(),
                output.display()
            );

            // Collect all memory keys and content from the dataset
            let mut all_memories: Vec<(String, String)> = Vec::new();
            for question in &questions {
                for (idx, session) in question.haystack_sessions.iter().enumerate() {
                    let session_id = question
                        .haystack_session_ids
                        .get(idx)
                        .map(|s| s.as_str())
                        .unwrap_or("unknown");

                    match strategy {
                        ingest::IngestStrategy::PerTurn => {
                            for (turn_idx, turn) in session.iter().enumerate() {
                                let key =
                                    format!("{session_id}:turn:{turn_idx}:{}", turn.role);
                                all_memories.push((key, turn.content.clone()));
                            }
                        }
                        ingest::IngestStrategy::PerSession => {
                            let content: String = session
                                .iter()
                                .map(|t| format!("{}: {}", t.role, t.content))
                                .collect::<Vec<_>>()
                                .join("\n");
                            let key = format!("{session_id}:session");
                            all_memories.push((key, content));
                        }
                    }
                }
            }

            // Dedup by key (same session may appear in multiple questions)
            let mut seen = std::collections::HashSet::new();
            all_memories.retain(|(key, _)| seen.insert(key.clone()));

            eprintln!(
                "Dataset: {} questions, {} unique memories to describe",
                questions.len(),
                all_memories.len()
            );

            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
            let generator =
                spectral_bench_accuracy::describe::AnthropicDescriber::new(api_key, model, base_url);

            let descriptions =
                spectral_bench_accuracy::describe::generate_descriptions_incremental(
                    &all_memories,
                    &existing,
                    regenerate,
                    &generator,
                    Some(&output),
                    100,
                )?;

            eprintln!(
                "\nDescriptions saved to {} ({} total)",
                output.display(),
                descriptions.len()
            );
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
