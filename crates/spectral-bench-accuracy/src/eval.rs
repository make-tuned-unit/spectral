//! Evaluation orchestration: full eval loop.

use crate::actor::Actor;
use crate::dataset::{Category, Question};
use crate::ingest::{self, IngestStrategy};
use crate::judge::Judge;
use crate::report::{EvalReport, RunStatus};
use crate::retrieval::{self, RetrievalConfig, RetrievalPath};
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
    /// Which retrieval path to use (tact or graph).
    pub retrieval_path: RetrievalPath,
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
            retrieval_path: RetrievalPath::default(),
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

/// Result of evaluating a single question.
struct SingleResult {
    correct: bool,
    predicted: String,
    memory_count: usize,
    memory_keys: Vec<String>,
    reasoning: Option<String>,
    duration_ms: u64,
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
        let questions_all = crate::dataset::load_dataset(&self.config.dataset_path)?;
        let questions = self.filter_questions(&questions_all);

        eprintln!(
            "Running {} questions (actor: {}, judge: {})",
            questions.len(),
            self.actor.name(),
            self.judge.name()
        );

        let mut report = EvalReport::new(self.actor.name(), self.judge.name());
        report.retrieval_path = match self.config.retrieval_path {
            RetrievalPath::Tact => "tact".into(),
            RetrievalPath::Graph => "graph".into(),
        };
        let pb = ProgressBar::new(questions.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );

        let checkpoint_path = self.config.work_dir.join("checkpoint.json");
        let completed = self.load_completed_ids(&checkpoint_path);
        let mut consecutive_errors: usize = 0;
        const MAX_CONSECUTIVE_ERRORS: usize = 3;

        for (idx, question) in questions.iter().enumerate() {
            if completed.contains(&question.question_id) {
                pb.inc(1);
                continue;
            }

            let category = match Category::from_question_type(&question.question_type) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("warn: skipping {} — {e}", question.question_id);
                    pb.inc(1);
                    continue;
                }
            };

            match self.eval_single(question, category) {
                Ok(r) => {
                    consecutive_errors = 0;
                    let answer_text = question.answer_text();
                    report.record(
                        &question.question_id,
                        category,
                        r.correct,
                        &question.question,
                        &r.predicted,
                        &answer_text,
                        r.reasoning,
                        r.memory_count,
                        r.memory_keys,
                        r.duration_ms,
                    );
                }
                Err(e) => {
                    consecutive_errors += 1;
                    eprintln!("[ERROR] {}: {e}", question.question_id);
                    let answer_text = question.answer_text();
                    report.record(
                        &question.question_id,
                        category,
                        false,
                        &question.question,
                        &format!("[error: {e}]"),
                        &answer_text,
                        Some(format!("API call failed: {e}")),
                        0,
                        Vec::new(),
                        0,
                    );

                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        eprintln!(
                            "[FATAL] {} consecutive errors. Halting run. Partial report saved.",
                            consecutive_errors
                        );
                        report.run_status = RunStatus::HaltedOnErrors { consecutive_errors };
                        break;
                    }
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
    fn eval_single(&self, question: &Question, category: Category) -> Result<SingleResult> {
        let start = std::time::Instant::now();
        let brain_dir = self
            .config
            .work_dir
            .join(format!("brain_{}", question.question_id));

        // Ingest
        let brain = ingest::ingest_question(question, &brain_dir, self.config.ingest_strategy)?;

        // Retrieve
        let memories = match self.config.retrieval_path {
            RetrievalPath::Tact => {
                retrieval::retrieve(&brain, &question.question, &self.config.retrieval)?
            }
            RetrievalPath::Graph => {
                retrieval::retrieve_graph(&brain, &question.question, &self.config.retrieval)?
            }
        };
        let memory_count = memories.len();
        // Extract keys from formatted "[date] [wing/hall] key: content" lines
        let memory_keys: Vec<String> = memories
            .iter()
            .filter_map(|m| {
                // Skip the two bracketed prefixes, then take the key before ": "
                let after_brackets = m.split("] ").last()?;
                after_brackets.split(": ").next().map(|k| k.to_string())
            })
            .collect();

        // Act
        let question_date = question.question_date.as_deref().unwrap_or("unknown");
        let predicted = self
            .actor
            .answer(&question.question, question_date, &memories)?;

        // Judge
        let answer_text = question.answer_text();
        let grade = self
            .judge
            .grade(&question.question, &predicted, &answer_text, category)?;

        // Clean up brain directory
        let _ = std::fs::remove_dir_all(&brain_dir);

        Ok(SingleResult {
            correct: grade.correct,
            predicted,
            memory_count,
            memory_keys,
            reasoning: grade.reasoning,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    fn filter_questions<'a>(&self, questions_all: &'a [Question]) -> Vec<&'a Question> {
        let mut questions: Vec<&Question> = questions_all.iter().collect();

        if let Some(ref cats) = self.config.categories {
            let cat_strs: HashSet<String> = cats.iter().map(|c| c.as_str().to_string()).collect();
            questions.retain(|q| {
                Category::from_question_type(&q.question_type)
                    .map(|cat| cat_strs.contains(cat.as_str()))
                    .unwrap_or(false)
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
                .results
                .iter()
                .map(|r| r.question_id.clone())
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
    use crate::actor::{Actor, MockActor};
    use crate::dataset::{Question, Turn};
    use crate::judge::MockJudge;

    /// Actor that always returns an error.
    struct FailingActor;
    impl Actor for FailingActor {
        fn answer(&self, _q: &str, _d: &str, _m: &[String]) -> anyhow::Result<String> {
            Err(anyhow::anyhow!("API returned 401: unauthorized"))
        }
        fn name(&self) -> &str {
            "failing"
        }
    }

    /// Actor that fails on the Nth call (0-indexed), succeeds otherwise.
    struct FailNthActor {
        fail_on: usize,
        call_count: std::sync::Mutex<usize>,
    }
    impl FailNthActor {
        fn new(fail_on: usize) -> Self {
            Self {
                fail_on,
                call_count: std::sync::Mutex::new(0),
            }
        }
    }
    impl Actor for FailNthActor {
        fn answer(&self, _q: &str, _d: &str, _m: &[String]) -> anyhow::Result<String> {
            let mut count = self.call_count.lock().unwrap();
            let current = *count;
            *count += 1;
            if current == self.fail_on {
                Err(anyhow::anyhow!("API returned 429: rate limited"))
            } else {
                Ok("test answer".into())
            }
        }
        fn name(&self) -> &str {
            "fail-nth"
        }
    }

    fn test_questions() -> Vec<Question> {
        vec![
            Question {
                question_id: "q1".into(),
                question_type: "multi-session".into(),
                question: "What is unknown?".into(),
                answer: serde_json::Value::String("I don't know".into()),
                question_date: Some("2023/05/30 (Tue) 23:40".into()),
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
                haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
            },
            Question {
                question_id: "q2".into(),
                question_type: "temporal-reasoning".into(),
                question: "What color is the car?".into(),
                answer: serde_json::Value::String("Red".into()),
                question_date: Some("2023/06/01 (Thu) 10:00".into()),
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
                haystack_dates: vec!["2023/03/01 (Wed) 12:00".into()],
            },
        ]
    }

    #[test]
    fn full_eval_with_mocks() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        std::fs::write(&ds_path, serde_json::to_string(&test_questions()).unwrap()).unwrap();

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
        std::fs::write(&ds_path, serde_json::to_string(&test_questions()).unwrap()).unwrap();

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
        assert_eq!(report.failures().len(), 2);
    }

    #[test]
    fn cost_estimate_reasonable() {
        let cost = estimate_cost(500);
        assert!(
            cost > 10.0 && cost < 100.0,
            "500 questions should cost $10-100, got ${cost}"
        );
    }

    #[test]
    fn unknown_question_type_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let qs = vec![Question {
            question_id: "q-unknown".into(),
            question_type: "bogus-category".into(),
            question: "Q?".into(),
            answer: serde_json::Value::String("A".into()),
            question_date: None,
            haystack_sessions: vec![vec![Turn {
                role: "user".into(),
                content: "Hello.".into(),
            }]],
            haystack_session_ids: vec!["s1".into()],
            haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
        }];
        std::fs::write(&ds_path, serde_json::to_string(&qs).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(MockActor::new("answer")),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        assert_eq!(report.total_questions, 0, "unknown type should be skipped");
    }

    fn make_n_questions(n: usize) -> Vec<Question> {
        (0..n)
            .map(|i| Question {
                question_id: format!("q{i}"),
                question_type: "multi-session".into(),
                question: format!("Question {i} about topic {i}?"),
                answer: serde_json::Value::String(format!("Answer {i}")),
                question_date: Some("2023/05/30 (Tue) 23:40".into()),
                haystack_sessions: vec![vec![Turn {
                    role: "user".into(),
                    content: format!("Content for question {i} about topic {i}."),
                }]],
                haystack_session_ids: vec![format!("s{i}")],
                haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
            })
            .collect()
    }

    #[test]
    fn eval_halts_on_consecutive_errors() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let qs = make_n_questions(5);
        std::fs::write(&ds_path, serde_json::to_string(&qs).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            checkpoint_interval: 100,
            ..Default::default()
        };

        let eval = AccuracyEval::new(
            config,
            Box::new(FailingActor),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        // Should halt after 3 consecutive errors, not process all 5
        assert_eq!(report.total_questions, 3);
        assert_eq!(
            report.run_status,
            RunStatus::HaltedOnErrors {
                consecutive_errors: 3
            }
        );
    }

    #[test]
    fn eval_continues_on_isolated_error() {
        let dir = tempfile::tempdir().unwrap();
        let ds_path = dir.path().join("dataset.json");
        let qs = make_n_questions(4);
        std::fs::write(&ds_path, serde_json::to_string(&qs).unwrap()).unwrap();

        let config = EvalConfig {
            dataset_path: ds_path,
            work_dir: dir.path().join("work"),
            checkpoint_interval: 100,
            ..Default::default()
        };

        // Fail on question index 1 only — the rest succeed
        let eval = AccuracyEval::new(
            config,
            Box::new(FailNthActor::new(1)),
            Box::new(MockJudge::always_pass()),
        );
        let report = eval.run().unwrap();
        // All 4 questions should be attempted
        assert_eq!(report.total_questions, 4);
        assert_eq!(report.run_status, RunStatus::Completed);
        // 3 correct, 1 failed
        assert_eq!(report.correct, 3);
        assert_eq!(report.failures().len(), 1);
        assert!(report.failures()[0].predicted.contains("[error:"));
    }
}
