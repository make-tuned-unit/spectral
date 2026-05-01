//! Evaluation orchestration: full eval loop.

use crate::actor::Actor;
use crate::dataset::{Category, Dataset, Question};
use crate::ingest::{self, IngestStrategy};
use crate::judge::Judge;
use crate::report::EvalReport;
use crate::retrieval::{self, RetrievalConfig};
use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Evaluation configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvalConfig {
    pub dataset_path: PathBuf,
    pub work_dir: PathBuf,
    pub max_questions: Option<usize>,
    pub categories: Option<Vec<Category>>,
    pub seed: u64,
    pub ingest_strategy: IngestStrategy,
    pub retrieval: RetrievalConfig,
    /// Save partial results every N questions.
    pub checkpoint_interval: usize,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            dataset_path: PathBuf::from("longmemeval_s.json"),
            work_dir: PathBuf::from("eval-work"),
            max_questions: None,
            categories: None,
            seed: 42,
            ingest_strategy: IngestStrategy::default(),
            retrieval: RetrievalConfig::default(),
            checkpoint_interval: 10,
        }
    }
}

/// Estimate the cost of running the eval.
pub fn estimate_cost(question_count: usize) -> f64 {
    // ~2 LLM calls per question (actor + judge), ~10K tokens each
    // Sonnet 4 pricing: ~$3/M input + $15/M output (rough Apr 2026)
    // ~10K input + ~0.5K output per call = ~$0.04 per call
    let calls = question_count * 2;
    calls as f64 * 0.04
}

/// The main evaluator.
pub struct AccuracyEval {
    config: EvalConfig,
    actor: Box<dyn Actor>,
    judge: Box<dyn Judge>,
}

impl AccuracyEval {
    pub fn new(config: EvalConfig, actor: Box<dyn Actor>, judge: Box<dyn Judge>) -> Self {
        Self {
            config,
            actor,
            judge,
        }
    }

    /// Run the full evaluation.
    pub fn run(&self) -> Result<EvalReport> {
        let dataset = crate::dataset::load_dataset(&self.config.dataset_path)?;
        let questions = self.filter_questions(&dataset);

        eprintln!(
            "Running {} questions (actor: {}, judge: {})",
            questions.len(),
            self.actor.name(),
            self.judge.name()
        );

        let mut report = EvalReport::new(self.actor.name(), self.judge.name());
        let pb = ProgressBar::new(questions.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );

        let checkpoint_path = self.config.work_dir.join("checkpoint.json");
        let completed = self.load_completed_ids(&checkpoint_path);

        for (idx, question) in questions.iter().enumerate() {
            if completed.contains(&question.question_id) {
                pb.inc(1);
                continue;
            }

            match self.eval_single(question) {
                Ok((correct, predicted, memory_count, reasoning)) => {
                    let category = Category::from_question_type(&question.question_type);
                    report.record(
                        &question.question_id,
                        category,
                        correct,
                        &question.question,
                        &predicted,
                        &question.answer,
                        reasoning,
                        memory_count,
                    );
                }
                Err(e) => {
                    eprintln!("Error on {}: {e}", question.question_id);
                    let category = Category::from_question_type(&question.question_type);
                    report.record(
                        &question.question_id,
                        category,
                        false,
                        &question.question,
                        &format!("[error: {e}]"),
                        &question.answer,
                        Some(format!("eval error: {e}")),
                        0,
                    );
                }
            }

            pb.inc(1);

            // Checkpoint
            if (idx + 1) % self.config.checkpoint_interval == 0 {
                let mut cp = report.clone();
                cp.finalize();
                let _ = crate::report::save_report(&cp, &checkpoint_path);
            }
        }

        pb.finish_with_message("done");
        report.finalize();
        Ok(report)
    }

    /// Run a single question: ingest, retrieve, act, judge.
    fn eval_single(&self, question: &Question) -> Result<(bool, String, usize, Option<String>)> {
        let brain_dir = self
            .config
            .work_dir
            .join(format!("brain_{}", question.question_id));

        // Ingest
        let brain = ingest::ingest_question(question, &brain_dir, self.config.ingest_strategy)?;

        // Retrieve
        let memories = retrieval::retrieve(&brain, &question.question, &self.config.retrieval)?;
        let memory_count = memories.len();

        // Act
        let predicted = self.actor.answer(&question.question, &memories)?;

        // Judge
        let category = Category::from_question_type(&question.question_type);
        let grade = self
            .judge
            .grade(&question.question, &predicted, &question.answer, category)?;

        // Clean up brain directory
        let _ = std::fs::remove_dir_all(&brain_dir);

        Ok((grade.correct, predicted, memory_count, grade.reasoning))
    }

    fn filter_questions<'a>(&self, dataset: &'a Dataset) -> Vec<&'a Question> {
        let mut questions: Vec<&Question> = dataset.questions.iter().collect();

        if let Some(ref cats) = self.config.categories {
            let cat_strs: HashSet<String> = cats.iter().map(|c| c.as_str().to_string()).collect();
            questions.retain(|q| {
                let cat = Category::from_question_type(&q.question_type);
                cat_strs.contains(cat.as_str())
            });
        }

        if let Some(max) = self.config.max_questions {
            questions.truncate(max);
        }

        questions
    }

    fn load_completed_ids(&self, checkpoint_path: &Path) -> HashSet<String> {
        if let Ok(report) = crate::report::load_report(checkpoint_path) {
            let mut ids: HashSet<String> = report
                .failures
                .iter()
                .map(|f| f.question_id.clone())
                .collect();
            // Also include questions that passed (not in failures)
            // We need to reconstruct from per_category totals — simpler to just re-run
            // For now, checkpoint means "these were attempted"
            ids.clear(); // TODO: proper resume tracking
            ids
        } else {
            HashSet::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::MockActor;
    use crate::dataset::{Dataset, Question, Turn};
    use crate::judge::MockJudge;

    fn test_dataset() -> Dataset {
        Dataset {
            questions: vec![
                Question {
                    question_id: "q1".into(),
                    question_type: "abstention".into(),
                    question: "What is unknown?".into(),
                    answer: "I don't know".into(),
                    haystack_sessions: vec![vec![
                        Turn {
                            role: "user".into(),
                            content: "Hello there.".into(),
                        },
                        Turn {
                            role: "assistant".into(),
                            content: "Hi!".into(),
                        },
                    ]],
                    haystack_session_ids: vec!["s1".into()],
                    haystack_dates: vec!["2024-01-15".into()],
                },
                Question {
                    question_id: "q2".into(),
                    question_type: "information_extraction".into(),
                    question: "What color is the car?".into(),
                    answer: "Red".into(),
                    haystack_sessions: vec![vec![
                        Turn {
                            role: "user".into(),
                            content: "My car is red.".into(),
                        },
                        Turn {
                            role: "assistant".into(),
                            content: "Nice car!".into(),
                        },
                    ]],
                    haystack_session_ids: vec!["s2".into()],
                    haystack_dates: vec!["2024-01-16".into()],
                },
            ],
        }
    }

    #[test]
    fn full_eval_with_mocks() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        std::fs::write(&ds_path, serde_json::to_string(&test_dataset()).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            max_questions: Some(2),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(MockActor::new("test answer")),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        assert_eq!(report.total_questions, 2);
        assert_eq!(report.correct, 2);
        assert!((report.overall_accuracy - 1.0).abs() < 0.001);
    }

    #[test]
    fn eval_records_failures() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        std::fs::write(&ds_path, serde_json::to_string(&test_dataset()).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            max_questions: Some(2),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(MockActor::new("wrong")),
            Box::new(MockJudge::always_fail()),
        );
        let report = eval.run().unwrap();
        assert_eq!(report.correct, 0);
        assert_eq!(report.failures.len(), 2);
    }

    #[test]
    fn cost_estimate_reasonable() {
        let cost = estimate_cost(500);
        assert!(
            cost > 10.0 && cost < 100.0,
            "500 questions should cost $10-100, got ${cost}"
        );
    }
}
