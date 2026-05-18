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
        Category::KnowledgeUpdate => {
            "The question tests whether the system recognizes updated information. \
             The answer is correct if it reflects the MOST RECENT information, not older versions."
        }
        Category::TemporalReasoning => {
            "The question requires reasoning about when events happened. \
             The answer is correct if the temporal aspect is accurately captured."
        }
        Category::MultiSession => {
            "The question requires synthesizing information across multiple conversation sessions.\n\n\
             COUNTING QUESTION PROTOCOL:\n\
             If this is a counting question (asks \"how many\", \"how much\", \"total\", or the ground truth is a number):\n\n\
             1. Extract the system's numerical answer and the ground truth number.\n\
             2. Compute delta = |system_count - ground_truth_count|.\n\
             3. If delta = 0: the answer is CORRECT.\n\
             4. If delta > 1: the answer is INCORRECT.\n\
             5. If delta = 1: apply the REASONING-AWARE TOLERANCE CHECK below.\n\n\
             REASONING-AWARE TOLERANCE CHECK (delta = 1 only):\n\
             Examine the system's full output (including <thinking> and <quotes> blocks) for EXPLICIT REASONING \
             about which items to include or exclude from the count. Look for these signals:\n\n\
             ACCEPT (mark correct) if the system:\n\
             - Explicitly names items it included or excluded and explains WHY\n\
             - Addresses categorization boundaries\n\
             - Reasons about whether specific items belong in the count\n\
             - Over-counted by 1 with explicit reasoning for including an additional item the GT excludes\n\n\
             Note: simply listing items in the count does not constitute reasoning. The system must show \
             DELIBERATION about whether items belong — either through <thinking> content addressing inclusion, \
             exhaustive <quotes> documentation of disputed items, or explicit statements about why an item \
             was included or excluded.\n\n\
             REJECT (mark incorrect) if the system:\n\
             - Simply lists fewer items than GT with no discussion of excluded items\n\
             - Shows no awareness that additional items might exist\n\
             - Does not engage with categorization boundaries\n\
             - Expresses no uncertainty or reasoning about the completeness of its count\n\n\
             DOLLAR AMOUNTS:\n\
             When the ground truth is a dollar amount (e.g., \"$2,500\"), treat delta=1 as exact match — \
             the tolerance is designed for unit counts, not dollar totals.\n\n\
             NON-COUNTING QUESTIONS:\n\
             If this is NOT a counting question, apply the standard rubric: the answer is correct if it \
             accurately combines relevant facts from different sessions, even if worded differently."
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

/// Judge that calls the Anthropic Messages API (or compatible endpoint).
pub struct AnthropicJudge {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::blocking::Client,
}

impl AnthropicJudge {
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            api_key,
            model,
            base_url,
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
        Ok(Self::new(
            api_key,
            "claude-sonnet-4-6".into(),
            "https://api.anthropic.com".into(),
        ))
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
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Judge API returned {}: {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        let json: serde_json::Value = resp.json()?;
        let text = json["content"][0]["text"].as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "Judge response missing content[0].text: {}",
                serde_json::to_string(&json).unwrap_or_default()
            )
        })?;

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
        let p = judge_prompt("Q?", "A", "A", Category::MultiSession);
        assert!(p.contains("multiple conversation sessions"));
        assert!(p.contains("COUNTING QUESTION PROTOCOL"));
        assert!(p.contains("REASONING-AWARE TOLERANCE CHECK"));

        let p2 = judge_prompt("Q?", "A", "A", Category::KnowledgeUpdate);
        assert!(p2.contains("MOST RECENT"));

        let p3 = judge_prompt("Q?", "A", "A", Category::SingleSessionUser);
        assert!(p3.contains("factual information"));

        let p4 = judge_prompt("Q?", "A", "A", Category::TemporalReasoning);
        assert!(p4.contains("temporal"));
    }

    #[test]
    fn mock_judge_always_pass() {
        let j = MockJudge::always_pass();
        let r = j.grade("Q", "A", "A", Category::MultiSession).unwrap();
        assert!(r.correct);
    }
}
