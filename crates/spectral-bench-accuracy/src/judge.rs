//! Judge LLM trait — grade predicted answers against ground truth.

use crate::dataset::Category;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Result of grading a single answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradeResult {
    pub correct: bool,
    pub reasoning: Option<String>,
}

/// Judge that evaluates predicted answers.
pub trait Judge: Send + Sync {
    fn grade(
        &self,
        question: &str,
        predicted: &str,
        ground_truth: &str,
        category: Category,
    ) -> Result<GradeResult>;
    fn name(&self) -> &str;
}

fn judge_prompt(question: &str, predicted: &str, ground_truth: &str, category: Category) -> String {
    let rubric = match category {
        Category::Abstention => {
            "The system should indicate it does not know or cannot determine the answer. \
             The answer is correct if the system abstains or says it doesn't know."
        }
        Category::KnowledgeUpdate => {
            "The question tests whether the system recognizes updated information. \
             The answer is correct if it reflects the MOST RECENT information, not older versions."
        }
        Category::TemporalReasoning => {
            "The question requires reasoning about when events happened. \
             The answer is correct if the temporal aspect is accurately captured."
        }
        _ => {
            "An answer is correct if it conveys the same factual information as the ground truth, \
             even if worded differently. Synonyms and paraphrasing are acceptable."
        }
    };

    format!(
        "You are grading a question-answering system's response.\n\n\
         Question: {question}\n\
         Ground truth: {ground_truth}\n\
         System answer: {predicted}\n\n\
         Rubric: {rubric}\n\n\
         Respond with JSON only: {{\"correct\": true|false, \"reasoning\": \"...\"}}"
    )
}

/// Judge that calls the Anthropic Messages API.
pub struct AnthropicJudge {
    api_key: String,
    model: String,
    client: reqwest::blocking::Client,
}

impl AnthropicJudge {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
        Ok(Self::new(api_key, "claude-sonnet-4-6".into()))
    }
}

impl Judge for AnthropicJudge {
    fn grade(
        &self,
        question: &str,
        predicted: &str,
        ground_truth: &str,
        category: Category,
    ) -> Result<GradeResult> {
        let prompt = judge_prompt(question, predicted, ground_truth, category);

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 512,
            "messages": [{"role": "user", "content": prompt}]
        });

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()?;

        let json: serde_json::Value = resp.json()?;
        let text = json["content"][0]["text"].as_str().unwrap_or("{}");

        // Extract JSON from response (may have surrounding text)
        let grade: GradeResult = if let Some(start) = text.find('{') {
            if let Some(end) = text.rfind('}') {
                serde_json::from_str(&text[start..=end]).unwrap_or(GradeResult {
                    correct: false,
                    reasoning: Some(format!("Failed to parse judge response: {text}")),
                })
            } else {
                GradeResult {
                    correct: false,
                    reasoning: Some(format!("No closing brace in judge response: {text}")),
                }
            }
        } else {
            GradeResult {
                correct: false,
                reasoning: Some(format!("No JSON in judge response: {text}")),
            }
        };

        Ok(grade)
    }

    fn name(&self) -> &str {
        &self.model
    }
}

/// Mock judge for testing.
pub struct MockJudge {
    always_correct: bool,
}

impl MockJudge {
    pub fn always_pass() -> Self {
        Self {
            always_correct: true,
        }
    }

    pub fn always_fail() -> Self {
        Self {
            always_correct: false,
        }
    }
}

impl Judge for MockJudge {
    fn grade(
        &self,
        _question: &str,
        _predicted: &str,
        _ground_truth: &str,
        _category: Category,
    ) -> Result<GradeResult> {
        Ok(GradeResult {
            correct: self.always_correct,
            reasoning: Some("mock".into()),
        })
    }

    fn name(&self) -> &str {
        "mock-judge"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_prompt_renders_per_category() {
        let p = judge_prompt("Q?", "A", "A", Category::Abstention);
        assert!(p.contains("abstains"));

        let p2 = judge_prompt("Q?", "A", "A", Category::KnowledgeUpdate);
        assert!(p2.contains("MOST RECENT"));

        let p3 = judge_prompt("Q?", "A", "A", Category::InformationExtraction);
        assert!(p3.contains("factual information"));
    }

    #[test]
    fn mock_judge_always_pass() {
        let j = MockJudge::always_pass();
        let r = j.grade("Q", "A", "A", Category::Abstention).unwrap();
        assert!(r.correct);
    }
}
