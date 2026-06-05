//! Replay actor + judge over a saved bench report with a new prompt template.
//!
//! Holds retrieval CONSTANT (uses frozen actor_context from the report) and
//! varies ONLY the actor prompt, so prompt deltas are cleanly attributable.

use anyhow::{Context, Result};
use std::collections::HashSet;

use crate::actor::{Actor, AnthropicActor};
use crate::judge::{AnthropicJudge, Judge};
use crate::report::{EvalReport, QuestionResult};
use crate::retrieval::QuestionType;

/// Configuration for a replay run.
pub struct ReplayConfig {
    /// Path to the saved bench report.
    pub report_path: std::path::PathBuf,
    /// Path to the actor prompt template file to test.
    pub actor_prompt: Option<std::path::PathBuf>,
    /// Optional: restrict to these question IDs (one per line).
    pub question_ids_path: Option<std::path::PathBuf>,
    /// Output path for the new report.
    pub output_path: std::path::PathBuf,
    /// If true, skip actor and only re-run judge on original predictions.
    pub judge_only: bool,
    /// Actor model name.
    pub actor_model: String,
    /// Judge model name.
    pub judge_model: String,
    /// Base URL for API calls.
    pub base_url: String,
}

/// Run the replay.
pub fn run_replay(config: &ReplayConfig) -> Result<EvalReport> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;

    let mut report: EvalReport =
        crate::report::load_report(&config.report_path).context("loading source report")?;

    // Load question ID filter if provided
    let id_filter: Option<HashSet<String>> = config
        .question_ids_path
        .as_ref()
        .map(|p| -> Result<HashSet<String>> {
            let content = std::fs::read_to_string(p)
                .with_context(|| format!("reading question IDs from {}", p.display()))?;
            Ok(content
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect())
        })
        .transpose()?;

    // Load custom prompt template if provided
    let custom_template: Option<String> = config
        .actor_prompt
        .as_ref()
        .map(|p| {
            std::fs::read_to_string(p)
                .with_context(|| format!("reading actor prompt from {}", p.display()))
        })
        .transpose()?;

    let actor = AnthropicActor::new(
        api_key.clone(),
        config.actor_model.clone(),
        config.base_url.clone(),
    );
    let judge = AnthropicJudge::new(api_key, config.judge_model.clone(), config.base_url.clone());

    // Count questions to replay
    let replay_count = report
        .results
        .iter()
        .filter(|r| should_replay(r, id_filter.as_ref()))
        .count();

    eprintln!(
        "Replaying {} of {} questions (judge_only={})",
        replay_count,
        report.results.len(),
        config.judge_only
    );

    let pb = indicatif::ProgressBar::new(replay_count as u64);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut replayed = 0;
    let mut replayed_correct = 0;
    let mut errors = 0;

    for result in report.results.iter_mut() {
        if !should_replay(result, id_filter.as_ref()) {
            continue;
        }

        if config.judge_only {
            // Re-judge the original prediction
            match judge.grade(
                &result.question,
                &result.predicted,
                &result.ground_truth,
                result.category,
            ) {
                Ok(grade) => {
                    result.replayed_predicted = Some(result.predicted.clone());
                    result.replayed_correct = Some(grade.correct);
                    result.replayed_judge_reasoning = grade.reasoning;
                    if grade.correct {
                        replayed_correct += 1;
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] {}: judge failed: {e}", result.question_id);
                    errors += 1;
                }
            }
        } else {
            // Replay actor + judge
            let actor_context = match &result.actor_context {
                Some(ctx) => ctx.clone(),
                None => {
                    eprintln!(
                        "[SKIP] {}: no actor_context in report (pre-replay-era report)",
                        result.question_id
                    );
                    errors += 1;
                    pb.inc(1);
                    continue;
                }
            };

            let question_date = result.question_date.as_deref().unwrap_or("unknown");

            // Build prompt: use custom template or the question's default
            let template = match &custom_template {
                Some(t) => t.clone(),
                None => {
                    let qtype = QuestionType::classify(&result.question);
                    qtype.prompt_content().to_string()
                }
            };

            let prompt = template
                .replace("{question_date}", question_date)
                .replace("{memories_text}", &actor_context)
                .replace("{question}", &result.question);

            match call_actor_raw(&actor, &prompt) {
                Ok(predicted) => {
                    // Judge the new prediction
                    match judge.grade(
                        &result.question,
                        &predicted,
                        &result.ground_truth,
                        result.category,
                    ) {
                        Ok(grade) => {
                            result.replayed_predicted = Some(predicted);
                            result.replayed_correct = Some(grade.correct);
                            result.replayed_judge_reasoning = grade.reasoning;
                            if grade.correct {
                                replayed_correct += 1;
                            }
                        }
                        Err(e) => {
                            eprintln!("[ERROR] {}: judge failed: {e}", result.question_id);
                            result.replayed_predicted = Some(predicted);
                            errors += 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] {}: actor failed: {e}", result.question_id);
                    errors += 1;
                }
            }
        }

        replayed += 1;
        pb.inc(1);
    }

    pb.finish_and_clear();

    eprintln!(
        "\nReplay complete: {replayed} replayed, {replayed_correct} correct, {errors} errors"
    );
    eprintln!(
        "Replay accuracy: {}/{} ({:.1}%)",
        replayed_correct,
        replayed - errors,
        if replayed > errors {
            replayed_correct as f64 / (replayed - errors) as f64 * 100.0
        } else {
            0.0
        }
    );

    // Save and return the modified report
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&config.output_path, &json)?;
    eprintln!("Report saved to {}", config.output_path.display());

    Ok(report)
}

/// Whether a result should be replayed.
fn should_replay(result: &QuestionResult, id_filter: Option<&HashSet<String>>) -> bool {
    // Skip network errors
    if result.predicted.starts_with("[error:") {
        return false;
    }
    // Apply ID filter
    if let Some(ids) = id_filter {
        return ids.contains(&result.question_id);
    }
    true
}

/// Call the actor with a pre-built prompt string (bypasses template selection).
fn call_actor_raw(actor: &AnthropicActor, prompt: &str) -> Result<String> {
    // Use the Actor trait's answer() with a dummy shape, but we need to
    // bypass it since we already built the prompt. Use direct API call instead.
    let body = serde_json::json!({
        "model": actor.name(),
        "max_tokens": 4096,
        "messages": [{"role": "user", "content": prompt}]
    });

    actor.call_raw(&body)
}
