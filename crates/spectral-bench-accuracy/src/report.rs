//! Results aggregation and output formatting.

use crate::dataset::Category;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_retrieval_path() -> String {
    "topk_fts".into()
}

/// Token usage from a single API call (from the Anthropic `usage` response field).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Per-question efficiency metrics.
///
/// All token counts come from the Anthropic API `usage` response field.
/// No tokenizer estimates are used anywhere. `None` means the response
/// lacked a usage field (counted in `missing_usage_count` on the report).
///
/// `system_tokens_per_query` = expansion_in + expansion_out + actor_in + actor_out.
/// Judge tokens are tracked separately because the judge is a bench
/// instrument, NOT part of the system under test.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EfficiencyMetrics {
    /// Expansion call input tokens (0 when --no-expand-queries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion_input_tokens: Option<u64>,
    /// Expansion call output tokens (0 when --no-expand-queries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion_output_tokens: Option<u64>,
    /// Actor call input tokens (includes full formatted context).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_input_tokens: Option<u64>,
    /// Actor call output tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_output_tokens: Option<u64>,
    /// Judge call input tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_input_tokens: Option<u64>,
    /// Judge call output tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_output_tokens: Option<u64>,
    /// Headline metric: expansion_in + expansion_out + actor_in + actor_out.
    /// Judge is excluded — it is bench instrument, not system cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_tokens_per_query: Option<u64>,
    /// Wall-clock ms for the full retrieval call (post-expansion, pre-actor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_wall_ms: Option<u64>,
    /// Wall-clock ms for the expansion API call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion_wall_ms: Option<u64>,
    /// Wall-clock ms for the actor API call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_wall_ms: Option<u64>,
    /// Estimated cost in USD for system calls (expansion + actor).
    /// `None` when model ID is unknown — never silently 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    /// Estimated cost in USD for the judge call (bench instrument, separate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_cost_usd: Option<f64>,
}

/// Aggregate efficiency statistics for a run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EfficiencyAggregate {
    pub mean_system_tokens: Option<f64>,
    pub median_system_tokens: Option<f64>,
    pub p95_system_tokens: Option<f64>,
    pub mean_retrieval_wall_ms: Option<f64>,
    pub median_retrieval_wall_ms: Option<f64>,
    pub p95_retrieval_wall_ms: Option<f64>,
    pub total_system_cost_usd: Option<f64>,
    pub total_judge_cost_usd: Option<f64>,
    pub missing_usage_count: usize,
}

/// Outcome classification for a question evaluation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeClass {
    /// Question was evaluated (answer may be right or wrong).
    #[default]
    Ok,
    /// Exhausted retries on a transient/transport error — no valid answer.
    TransportFailure,
    /// Hit a non-retryable auth/validation error (401/403/400).
    AuthFailure,
}

/// Whether the eval run completed or halted early.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    HaltedOnErrors { consecutive_errors: usize },
}

/// Per-category accuracy statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryStats {
    pub category: Category,
    pub total: usize,
    pub correct: usize,
    pub accuracy: f64,
}

/// Per-question strategy routing telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyTelemetry {
    pub shape: String,
    pub prompt_template: String,
    pub retrieval_path_chosen: String,
}

/// Detailed result for a single evaluated question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionResult {
    pub question_id: String,
    pub category: Category,
    pub question: String,
    pub ground_truth: String,
    pub predicted: String,
    pub correct: bool,
    pub judge_reasoning: Option<String>,
    pub retrieved_memory_count: usize,
    /// Keys of the top retrieved memories (not full content).
    pub retrieved_memory_keys: Vec<String>,
    /// Wall-clock time for this question in milliseconds.
    pub duration_ms: u64,
    /// Cascade telemetry (populated when --use-cascade is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cascade_telemetry: Option<crate::retrieval::CascadeTelemetry>,
    /// Strategy routing telemetry (populated when shape routing is active).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_telemetry: Option<StrategyTelemetry>,
    /// Number of retries needed (0 = clean first-try success).
    #[serde(default)]
    pub retry_count: u32,
    /// Outcome classification: ok, transport_failure, or auth_failure.
    #[serde(default)]
    pub outcome_class: OutcomeClass,
    /// Rendered memories text passed to the actor (for replay).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_context: Option<String>,
    /// Question date string used in the actor prompt (for replay).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_date: Option<String>,
    /// Replayed actor answer (populated by --replay-actor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayed_predicted: Option<String>,
    /// Replayed judge verdict (populated by --replay-actor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayed_correct: Option<bool>,
    /// Replayed judge reasoning (populated by --replay-actor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayed_judge_reasoning: Option<String>,
    /// Per-question efficiency metrics (tokens, latency, cost).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efficiency: Option<EfficiencyMetrics>,
}

/// Full evaluation report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    pub spectral_version: String,
    pub actor_name: String,
    pub judge_name: String,
    /// Which retrieval path was used ("tact" or "graph").
    #[serde(default = "default_retrieval_path")]
    pub retrieval_path: String,
    /// Fingerprint of the run configuration (question filter, routing, and
    /// SPECTRAL_* env levers). A checkpoint is only resumable by a run whose
    /// fingerprint matches — prevents A/B arms sharing a work_dir from
    /// silently inheriting each other's results.
    #[serde(default)]
    pub config_fingerprint: String,
    pub total_questions: usize,
    pub correct: usize,
    pub overall_accuracy: f64,
    /// Questions that completed cleanly on first try.
    #[serde(default)]
    pub clean: usize,
    /// Questions recovered after retry.
    #[serde(default)]
    pub recovered_after_retry: usize,
    /// Questions lost to transport/connection failures (excluded from accuracy).
    #[serde(default)]
    pub transport_failures: usize,
    /// Questions lost to auth failures (excluded from accuracy).
    #[serde(default)]
    pub auth_failures: usize,
    pub per_category: HashMap<String, CategoryStats>,
    /// Detailed per-question results.
    pub results: Vec<QuestionResult>,
    pub run_status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub duration_seconds: u64,
    /// Aggregate efficiency metrics for the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efficiency: Option<EfficiencyAggregate>,
    /// Per-category efficiency breakdowns.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub efficiency_per_category: HashMap<String, EfficiencyAggregate>,
}

impl EvalReport {
    /// Create a new empty report.
    pub fn new(actor_name: &str, judge_name: &str) -> Self {
        Self {
            spectral_version: env!("CARGO_PKG_VERSION").into(),
            actor_name: actor_name.into(),
            judge_name: judge_name.into(),
            retrieval_path: "tact".into(),
            config_fingerprint: String::new(),
            total_questions: 0,
            correct: 0,
            overall_accuracy: 0.0,
            clean: 0,
            recovered_after_retry: 0,
            transport_failures: 0,
            auth_failures: 0,
            per_category: HashMap::new(),
            results: Vec::new(),
            run_status: RunStatus::Completed,
            started_at: Utc::now(),
            completed_at: Utc::now(),
            duration_seconds: 0,
            efficiency: None,
            efficiency_per_category: HashMap::new(),
        }
    }

    /// Record a graded result.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        question_id: &str,
        category: Category,
        correct: bool,
        question: &str,
        predicted: &str,
        ground_truth: &str,
        judge_reasoning: Option<String>,
        memory_count: usize,
        memory_keys: Vec<String>,
        duration_ms: u64,
        cascade_telemetry: Option<crate::retrieval::CascadeTelemetry>,
        strategy_telemetry: Option<StrategyTelemetry>,
        retry_count: u32,
        outcome_class: OutcomeClass,
        actor_context: Option<String>,
        question_date: Option<String>,
        efficiency: Option<EfficiencyMetrics>,
    ) {
        self.total_questions += 1;

        // Track outcome summary
        match outcome_class {
            OutcomeClass::Ok => {
                if retry_count > 0 {
                    self.recovered_after_retry += 1;
                } else {
                    self.clean += 1;
                }
                if correct {
                    self.correct += 1;
                }
            }
            OutcomeClass::TransportFailure => {
                self.transport_failures += 1;
            }
            OutcomeClass::AuthFailure => {
                self.auth_failures += 1;
            }
        }

        // Only count in per-category accuracy if the question was actually evaluated
        if outcome_class == OutcomeClass::Ok {
            let cat_key = category.as_str().to_string();
            let entry = self
                .per_category
                .entry(cat_key)
                .or_insert_with(|| CategoryStats {
                    category,
                    total: 0,
                    correct: 0,
                    accuracy: 0.0,
                });
            entry.total += 1;
            if correct {
                entry.correct += 1;
            }
            entry.accuracy = entry.correct as f64 / entry.total as f64;
        }

        self.results.push(QuestionResult {
            question_id: question_id.into(),
            category,
            question: question.into(),
            ground_truth: ground_truth.into(),
            predicted: predicted.into(),
            correct,
            judge_reasoning,
            retrieved_memory_count: memory_count,
            retrieved_memory_keys: memory_keys,
            duration_ms,
            cascade_telemetry,
            strategy_telemetry,
            retry_count,
            outcome_class,
            actor_context,
            question_date,
            replayed_predicted: None,
            replayed_correct: None,
            replayed_judge_reasoning: None,
            efficiency,
        });
    }

    /// Returns failed results (correct == false).
    pub fn failures(&self) -> Vec<&QuestionResult> {
        self.results.iter().filter(|r| !r.correct).collect()
    }

    /// Finalize the report (accuracy + efficiency aggregates).
    pub fn finalize(&mut self) {
        self.completed_at = Utc::now();
        self.duration_seconds = (self.completed_at - self.started_at).num_seconds() as u64;
        // Accuracy denominator excludes transport/auth failures
        let evaluated = self.total_questions - self.transport_failures - self.auth_failures;
        self.overall_accuracy = if evaluated > 0 {
            self.correct as f64 / evaluated as f64
        } else {
            0.0
        };

        // Compute efficiency aggregates
        self.efficiency = Some(compute_efficiency_aggregate(&self.results));

        // Per-category efficiency
        let mut by_cat: HashMap<String, Vec<&QuestionResult>> = HashMap::new();
        for r in &self.results {
            by_cat
                .entry(r.category.as_str().to_string())
                .or_default()
                .push(r);
        }
        for (cat, results) in &by_cat {
            self.efficiency_per_category
                .insert(cat.clone(), compute_efficiency_aggregate_refs(results));
        }
    }

    /// Human-readable summary.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push("=== LongMemEval_S Evaluation Report ===".to_string());
        lines.push(format!("Spectral: {}", self.spectral_version));
        lines.push(format!(
            "Actor: {}  |  Judge: {}",
            self.actor_name, self.judge_name
        ));
        let evaluated = self.total_questions - self.transport_failures - self.auth_failures;
        lines.push(format!(
            "Overall: {}/{} ({:.1}%)",
            self.correct,
            evaluated,
            self.overall_accuracy * 100.0
        ));
        lines.push(format!("Duration: {}s", self.duration_seconds));
        if self.transport_failures > 0 || self.auth_failures > 0 || self.recovered_after_retry > 0 {
            lines.push(format!(
                "Reliability: {} clean, {} recovered, {} transport failures, {} auth failures",
                self.clean, self.recovered_after_retry, self.transport_failures, self.auth_failures
            ));
        }
        lines.push(String::new());

        lines.push(format!(
            "{:<30} {:>5} {:>5} {:>8}",
            "Category", "Total", "Pass", "Accuracy"
        ));
        lines.push("-".repeat(55));
        for cat in Category::all() {
            if let Some(stats) = self.per_category.get(cat.as_str()) {
                lines.push(format!(
                    "{:<30} {:>5} {:>5} {:>7.1}%",
                    cat.as_str(),
                    stats.total,
                    stats.correct,
                    stats.accuracy * 100.0
                ));
            }
        }

        // Efficiency summary
        if let Some(ref eff) = self.efficiency {
            lines.push(String::new());
            lines.push("=== Efficiency (judge excluded from system cost) ===".to_string());
            if let Some(mean) = eff.mean_system_tokens {
                lines.push(format!(
                    "system_tokens_per_query:  mean={:.0}  median={:.0}  p95={:.0}",
                    mean,
                    eff.median_system_tokens.unwrap_or(0.0),
                    eff.p95_system_tokens.unwrap_or(0.0),
                ));
            }
            if let Some(mean) = eff.mean_retrieval_wall_ms {
                lines.push(format!(
                    "retrieval_wall_ms:        mean={:.0}  median={:.0}  p95={:.0}",
                    mean,
                    eff.median_retrieval_wall_ms.unwrap_or(0.0),
                    eff.p95_retrieval_wall_ms.unwrap_or(0.0),
                ));
            }
            if let Some(sys_cost) = eff.total_system_cost_usd {
                lines.push(format!("total system cost:        ${sys_cost:.4}"));
            }
            if let Some(judge_cost) = eff.total_judge_cost_usd {
                lines.push(format!("total judge cost:         ${judge_cost:.4}"));
            }
            if eff.missing_usage_count > 0 {
                lines.push(format!(
                    "WARNING: {} API responses lacked usage field",
                    eff.missing_usage_count
                ));
            }
        }

        let failures = self.failures();
        if !failures.is_empty() {
            lines.push(String::new());
            lines.push(format!("Failures: {}", failures.len()));
            for f in failures.iter().take(20) {
                lines.push(format!("  {} [{}]", f.question_id, f.category));
            }
            if failures.len() > 20 {
                lines.push(format!("  ... and {} more", failures.len() - 20));
            }
        }

        lines.join("\n")
    }
}

/// Compute percentile from a sorted slice.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (p / 100.0 * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Compute efficiency aggregate from owned QuestionResult slice.
fn compute_efficiency_aggregate(results: &[QuestionResult]) -> EfficiencyAggregate {
    let refs: Vec<&QuestionResult> = results.iter().collect();
    compute_efficiency_aggregate_refs(&refs)
}

/// Compute efficiency aggregate from QuestionResult references.
fn compute_efficiency_aggregate_refs(results: &[&QuestionResult]) -> EfficiencyAggregate {
    let mut sys_tokens: Vec<f64> = Vec::new();
    let mut ret_wall: Vec<f64> = Vec::new();
    let mut total_sys_cost = 0.0_f64;
    let mut total_judge_cost = 0.0_f64;
    let mut has_sys_cost = false;
    let mut has_judge_cost = false;
    let mut missing = 0usize;

    for r in results {
        let eff = match &r.efficiency {
            Some(e) => e,
            None => continue,
        };
        if let Some(st) = eff.system_tokens_per_query {
            sys_tokens.push(st as f64);
        }
        if let Some(rw) = eff.retrieval_wall_ms {
            ret_wall.push(rw as f64);
        }
        if let Some(c) = eff.estimated_cost_usd {
            total_sys_cost += c;
            has_sys_cost = true;
        }
        if let Some(c) = eff.judge_cost_usd {
            total_judge_cost += c;
            has_judge_cost = true;
        }
        // Count missing usage: actor is the minimum required signal
        if eff.actor_input_tokens.is_none() {
            missing += 1;
        }
    }

    sys_tokens.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ret_wall.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mean_sys = if sys_tokens.is_empty() {
        None
    } else {
        Some(sys_tokens.iter().sum::<f64>() / sys_tokens.len() as f64)
    };
    let median_sys = if sys_tokens.is_empty() {
        None
    } else {
        Some(percentile(&sys_tokens, 50.0))
    };
    let p95_sys = if sys_tokens.is_empty() {
        None
    } else {
        Some(percentile(&sys_tokens, 95.0))
    };

    let mean_ret = if ret_wall.is_empty() {
        None
    } else {
        Some(ret_wall.iter().sum::<f64>() / ret_wall.len() as f64)
    };
    let median_ret = if ret_wall.is_empty() {
        None
    } else {
        Some(percentile(&ret_wall, 50.0))
    };
    let p95_ret = if ret_wall.is_empty() {
        None
    } else {
        Some(percentile(&ret_wall, 95.0))
    };

    EfficiencyAggregate {
        mean_system_tokens: mean_sys,
        median_system_tokens: median_sys,
        p95_system_tokens: p95_sys,
        mean_retrieval_wall_ms: mean_ret,
        median_retrieval_wall_ms: median_ret,
        p95_retrieval_wall_ms: p95_ret,
        total_system_cost_usd: if has_sys_cost {
            Some(total_sys_cost)
        } else {
            None
        },
        total_judge_cost_usd: if has_judge_cost {
            Some(total_judge_cost)
        } else {
            None
        },
        missing_usage_count: missing,
    }
}

/// Save a report to JSON.
pub fn save_report(report: &EvalReport, path: &std::path::Path) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Load a previously saved report.
pub fn load_report(path: &std::path::Path) -> anyhow::Result<EvalReport> {
    let json = std::fs::read_to_string(path)?;
    let report: EvalReport = serde_json::from_str(&json)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_aggregation_computes_accuracy() {
        let mut report = EvalReport::new("mock", "mock-judge");
        report.record(
            "q1",
            Category::MultiSession,
            true,
            "Q?",
            "A",
            "A",
            None,
            5,
            vec!["k1".into()],
            100,
            None,
            None,
            0,
            OutcomeClass::Ok,
            None,
            None,
            None,
        );
        report.record(
            "q2",
            Category::MultiSession,
            false,
            "Q2?",
            "B",
            "C",
            Some("wrong answer".into()),
            3,
            vec!["k2".into(), "k3".into()],
            200,
            None,
            None,
            0,
            OutcomeClass::Ok,
            None,
            None,
            None,
        );
        report.record(
            "q3",
            Category::TemporalReasoning,
            true,
            "Q3?",
            "X",
            "X",
            None,
            10,
            vec![],
            50,
            None,
            None,
            0,
            OutcomeClass::Ok,
            None,
            None,
            None,
        );
        report.finalize();

        assert_eq!(report.total_questions, 3);
        assert_eq!(report.correct, 2);
        assert!((report.overall_accuracy - 2.0 / 3.0).abs() < 0.001);
        assert_eq!(report.per_category["multi-session"].total, 2);
        assert_eq!(report.per_category["multi-session"].correct, 1);
        assert_eq!(report.results.len(), 3);
        assert_eq!(report.failures().len(), 1);
        assert_eq!(report.failures()[0].question_id, "q2");
    }

    #[test]
    fn question_result_serializes_all_fields() {
        let qr = QuestionResult {
            question_id: "q42".into(),
            category: Category::TemporalReasoning,
            question: "When did it happen?".into(),
            ground_truth: "Tuesday".into(),
            predicted: "Wednesday".into(),
            correct: false,
            judge_reasoning: Some("Off by one day".into()),
            retrieved_memory_count: 7,
            retrieved_memory_keys: vec!["s1:turn:0:user".into(), "s2:turn:1:assistant".into()],
            duration_ms: 1234,
            cascade_telemetry: None,
            strategy_telemetry: None,
            retry_count: 0,
            outcome_class: OutcomeClass::Ok,
            actor_context: None,
            question_date: None,
            replayed_predicted: None,
            replayed_correct: None,
            replayed_judge_reasoning: None,
            efficiency: None,
        };
        let json = serde_json::to_string(&qr).unwrap();
        assert!(json.contains("\"question_id\":\"q42\""));
        assert!(json.contains("\"category\":\"temporal-reasoning\""));
        assert!(json.contains("\"ground_truth\":\"Tuesday\""));
        assert!(json.contains("\"predicted\":\"Wednesday\""));
        assert!(json.contains("\"correct\":false"));
        assert!(json.contains("\"judge_reasoning\":\"Off by one day\""));
        assert!(json.contains("\"retrieved_memory_count\":7"));
        assert!(json.contains("\"retrieved_memory_keys\""));
        assert!(json.contains("\"duration_ms\":1234"));
    }
}
