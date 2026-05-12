//! Results aggregation and output formatting.

use crate::dataset::Category;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_retrieval_path() -> String {
    "topk_fts".into()
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
    pub total_questions: usize,
    pub correct: usize,
    pub overall_accuracy: f64,
    pub per_category: HashMap<String, CategoryStats>,
    /// Detailed per-question results.
    pub results: Vec<QuestionResult>,
    pub run_status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub duration_seconds: u64,
}

impl EvalReport {
    /// Create a new empty report.
    pub fn new(actor_name: &str, judge_name: &str) -> Self {
        Self {
            spectral_version: env!("CARGO_PKG_VERSION").into(),
            actor_name: actor_name.into(),
            judge_name: judge_name.into(),
            retrieval_path: "tact".into(),
            total_questions: 0,
            correct: 0,
            overall_accuracy: 0.0,
            per_category: HashMap::new(),
            results: Vec::new(),
            run_status: RunStatus::Completed,
            started_at: Utc::now(),
            completed_at: Utc::now(),
            duration_seconds: 0,
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
    ) {
        self.total_questions += 1;
        if correct {
            self.correct += 1;
        }

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
        });
    }

    /// Returns failed results (correct == false).
    pub fn failures(&self) -> Vec<&QuestionResult> {
        self.results.iter().filter(|r| !r.correct).collect()
    }

    /// Finalize the report.
    pub fn finalize(&mut self) {
        self.completed_at = Utc::now();
        self.duration_seconds = (self.completed_at - self.started_at).num_seconds() as u64;
        self.overall_accuracy = if self.total_questions > 0 {
            self.correct as f64 / self.total_questions as f64
        } else {
            0.0
        };
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
        lines.push(format!(
            "Overall: {}/{} ({:.1}%)",
            self.correct,
            self.total_questions,
            self.overall_accuracy * 100.0
        ));
        lines.push(format!("Duration: {}s", self.duration_seconds));
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
