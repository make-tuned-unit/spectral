//! Judge LLM trait — grade predicted answers against ground truth.

use crate::dataset::Category;
use crate::report::TokenUsage;
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
    ) -> Result<(GradeResult, Option<TokenUsage>)>;
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
             even if worded differently. Synonyms and paraphrasing are acceptable.\n\n\
             SUPERSET ANSWERS:\n\
             If the system answer includes the ground truth PLUS additional detail, apply these rules:\n\n\
             ACCEPT the answer if:\n\
             - The ground truth is clearly present within the system answer\n\
             - The additional content is topically related to the question (e.g., answering \
               \"what gift did I buy?\" with \"yellow dress and matching earrings\" when GT is \
               \"yellow dress\" — earrings are topically related to gift-buying)\n\
             - A reasonable reader would say \"this answers the question, with extra context\"\n\n\
             REJECT the answer if:\n\
             - The additional content contradicts the ground truth\n\
             - The additional content is topically unrelated to the question\n\
             - The system answer buries the ground truth in so much noise that it is not clearly \
               identifiable as an asserted fact (e.g., \"Maybe yellow dress. Could be blue.\" — \
               ambiguity undermines the assertion)\n\
             - The system answer does not actually contain the ground truth information"
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

/// Extract token usage from the Anthropic API response JSON.
fn extract_usage(json: &serde_json::Value) -> Option<TokenUsage> {
    let usage = json.get("usage")?;
    Some(TokenUsage {
        input_tokens: usage.get("input_tokens").and_then(|v| v.as_u64()),
        output_tokens: usage.get("output_tokens").and_then(|v| v.as_u64()),
    })
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
    ) -> Result<(GradeResult, Option<TokenUsage>)> {
        let prompt = judge_prompt(question, predicted, ground_truth, category);

        let body = serde_json::json!({
            // 512 truncated verbose/thinking-model judges (e.g. sonnet-5) mid-JSON,
            // losing the closing brace -> parse failure -> false "incorrect".
            "model": self.model,
            "max_tokens": 2048,
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
        let usage = extract_usage(&json);
        let text = crate::actor::extract_text(&json).ok_or_else(|| {
            anyhow::anyhow!(
                "Judge response missing a text block: {}",
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

        Ok((grade, usage))
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
    ) -> Result<(GradeResult, Option<TokenUsage>)> {
        Ok((
            GradeResult {
                correct: self.always_correct,
                reasoning: Some("mock".into()),
            },
            None,
        ))
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
        let (r, usage) = j.grade("Q", "A", "A", Category::MultiSession).unwrap();
        assert!(r.correct);
        assert!(usage.is_none());
    }

    #[test]
    fn default_rubric_contains_superset_rules() {
        let p = judge_prompt(
            "What did I buy?",
            "yellow dress and earrings",
            "yellow dress",
            Category::SingleSessionUser,
        );
        assert!(
            p.contains("SUPERSET ANSWERS"),
            "default rubric should contain superset rules"
        );
        assert!(
            p.contains("topically related"),
            "should mention topical relevance"
        );
        assert!(
            p.contains("contradicts"),
            "should mention contradiction rejection"
        );
    }

    #[test]
    fn superset_rubric_not_in_multi_session() {
        // MultiSession has its own counting protocol — superset rules should not appear
        let p = judge_prompt("How many X?", "3", "3", Category::MultiSession);
        assert!(
            !p.contains("SUPERSET ANSWERS"),
            "multi-session should use counting protocol, not superset rubric"
        );
    }

    #[test]
    fn superset_rubric_not_in_knowledge_update() {
        let p = judge_prompt("What is X?", "A", "A", Category::KnowledgeUpdate);
        assert!(
            !p.contains("SUPERSET ANSWERS"),
            "knowledge-update has its own recency rubric"
        );
    }

    #[test]
    fn superset_rubric_not_in_temporal() {
        let p = judge_prompt("When?", "A", "A", Category::TemporalReasoning);
        assert!(
            !p.contains("SUPERSET ANSWERS"),
            "temporal has its own rubric"
        );
    }

    #[test]
    fn superset_rubric_applies_to_all_default_categories() {
        for cat in [
            Category::SingleSessionUser,
            Category::SingleSessionAssistant,
            Category::SingleSessionPreference,
        ] {
            let p = judge_prompt("Q?", "A", "A", cat);
            assert!(
                p.contains("SUPERSET ANSWERS"),
                "category {:?} should use default rubric with superset rules",
                cat
            );
        }
    }
}
