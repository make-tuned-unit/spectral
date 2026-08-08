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

/// Extract the **first complete** JSON object from a judge response (R21).
///
/// The judge is asked for JSON and usually complies, but sometimes emits a
/// valid object followed by commentary. The previous extraction took the span
/// from the first `{` to the **last** `}`, so any trailing prose containing a
/// brace was pulled into the slice and `serde_json` rejected the whole thing
/// with `trailing characters` — scoring the question **incorrect** even though
/// the judge's own verdict was `"correct": true`. Measured on the BM25 LoCoMo
/// baseline: 4/1438 questions, 3 of them false negatives.
///
/// This scans for balanced braces instead, and is string-aware so that braces
/// inside a `reasoning` string (and `\"` escapes within it) do not move the
/// depth counter. Returns the first balanced object, or `None`.
///
/// Deliberately NOT tolerant of anything else: a response with no balanced
/// object is still a judge failure, excluded from the accuracy denominator
/// rather than silently scored wrong.
pub(crate) fn first_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for i in start..bytes.len() {
        let c = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                // `checked_sub`, not `-=`: if a stray quote in the prose
                // desynchronised the string state, a `}` can arrive at depth 0.
                // Underflowing would panic and kill a multi-hour run; giving up
                // here just records a judge failure, which is the safe
                // direction (excluded from the denominator, never scored wrong).
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
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
        Category::SingleSessionPreference => {
            "The question asks for a response that respects a stated user preference. \
             The ground truth describes the preference(s) the response must honor. \
             The answer is correct if it substantively incorporates or complies with those \
             preference(s). It is incorrect if it ignores them, contradicts them, or gives \
             generic advice that could have been written without knowing them — do NOT grade \
             this as a fact-recall question."
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
         ABSTENTION RULE (applies on top of the rubric, in both directions):\n\
         - If the ground truth itself states the information was not mentioned or is \
           not available, then a system answer that abstains (says it does not know, \
           has no record, or that the information was not mentioned) is CORRECT, and a \
           system answer asserting a specific value is INCORRECT.\n\
         - Otherwise — the ground truth is a specific answer — a system answer that \
           abstains or says it does not know is INCORRECT, no matter how reasonable \
           the abstention sounds. Abstention is never a match for a specific fact.\n\n\
         Respond with JSON only: {{\"correct\": true|false, \"reasoning\": \"...\"}}"
    )
}

/// Fingerprint of the grading rubrics. Changing any rubric text changes this
/// hash; it feeds the run config fingerprint and the report, so runs graded
/// under different rubrics are never silently treated as comparable/resumable.
pub fn rubric_fingerprint() -> String {
    let all: String = [
        Category::MultiSession,
        Category::TemporalReasoning,
        Category::KnowledgeUpdate,
        Category::SingleSessionUser,
        Category::SingleSessionAssistant,
        Category::SingleSessionPreference,
    ]
    .iter()
    .map(|c| judge_prompt("", "", "", *c))
    .collect();
    blake3::hash(all.as_bytes()).to_hex().to_string()
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
            // Large grading prompts can exceed reqwest's default client
            // timeout; a slow-but-valid response must not register as a
            // transport failure. Matches the OpenAI-compat clients.
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .expect("build reqwest client"),
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
            // Deterministic grading: pin temperature so the same (answer, gold)
            // pair grades identically across runs and A/B arms.
            "temperature": 0,
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

        // Extract JSON from response (may have surrounding text). A response we
        // cannot parse is a judge FAILURE, not a wrong answer — returning Err
        // lets the retry layer re-ask, and terminal failures are excluded from
        // the accuracy denominator instead of silently scored incorrect.
        let grade: GradeResult = match first_json_object(&text) {
            Some(obj) => serde_json::from_str(obj).map_err(|err| {
                anyhow::anyhow!(
                    "judge parse failure: {err}: {}",
                    text.chars().take(300).collect::<String>()
                )
            })?,
            None => {
                return Err(anyhow::anyhow!(
                    "judge parse failure: no JSON object in response: {}",
                    text.chars().take(300).collect::<String>()
                ))
            }
        };

        Ok((grade, usage))
    }

    fn name(&self) -> &str {
        &self.model
    }
}

/// Judge that calls an OpenAI-compatible `/v1/chat/completions` endpoint (local
/// model). Mirrors `AnthropicJudge` grading, for the fully-local accuracy loop.
pub struct OpenAiJudge {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::blocking::Client,
}

impl OpenAiJudge {
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            api_key,
            model,
            base_url,
            // See OpenAiActor: local prompt-eval can exceed the default
            // client timeout on large contexts.
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .expect("build reqwest client"),
        }
    }
}

impl Judge for OpenAiJudge {
    fn grade(
        &self,
        question: &str,
        predicted: &str,
        ground_truth: &str,
        category: Category,
    ) -> Result<(GradeResult, Option<TokenUsage>)> {
        let prompt = judge_prompt(question, predicted, ground_truth, category);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 2048,
            "temperature": 0,
            "stream": false,
        });
        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()?;
        let status = resp.status();
        if !status.is_success() {
            let b = resp.text().unwrap_or_default();
            return Err(anyhow::anyhow!(
                "OpenAI-compat judge returned {}: {}",
                status,
                b.chars().take(500).collect::<String>()
            ));
        }
        let json: serde_json::Value = resp.json()?;
        let usage = json.get("usage").map(|u| TokenUsage {
            input_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()),
            output_tokens: u.get("completion_tokens").and_then(|v| v.as_u64()),
        });
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("OpenAI-compat judge missing content"))?
            .to_string();
        // See AnthropicJudge: unparseable judge output is a failure, not a
        // wrong answer. Same first-complete-object extraction (R21).
        let grade: GradeResult = match first_json_object(&text) {
            Some(obj) => serde_json::from_str(obj).map_err(|err| {
                anyhow::anyhow!(
                    "judge parse failure: {err}: {}",
                    text.chars().take(300).collect::<String>()
                )
            })?,
            None => {
                return Err(anyhow::anyhow!(
                    "judge parse failure: no JSON object in response: {}",
                    text.chars().take(300).collect::<String>()
                ))
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
        ] {
            let p = judge_prompt("Q?", "A", "A", cat);
            assert!(
                p.contains("SUPERSET ANSWERS"),
                "category {:?} should use default rubric with superset rules",
                cat
            );
        }
    }

    #[test]
    fn preference_rubric_is_not_fact_recall() {
        let p = judge_prompt("Q?", "A", "A", Category::SingleSessionPreference);
        assert!(p.contains("stated user preference"));
        assert!(!p.contains("SUPERSET ANSWERS"));
    }

    #[test]
    fn abstention_rule_present_in_every_rubric() {
        for cat in [
            Category::MultiSession,
            Category::TemporalReasoning,
            Category::KnowledgeUpdate,
            Category::SingleSessionUser,
            Category::SingleSessionAssistant,
            Category::SingleSessionPreference,
        ] {
            let p = judge_prompt("Q?", "A", "A", cat);
            assert!(p.contains("ABSTENTION RULE"), "missing in {cat:?}");
        }
    }

    #[test]
    fn rubric_fingerprint_is_stable_and_nonempty() {
        let a = rubric_fingerprint();
        let b = rubric_fingerprint();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    // ── R21: first-complete-object extraction ──────────────────────────
    //
    // The defect these pin cost 4/1438 questions on the BM25 LoCoMo baseline,
    // 3 of them scored incorrect while the judge had said `"correct": true`.
    // Every case below FAILS under the old `find('{')`..`rfind('}')` span.

    fn parse(text: &str) -> Option<GradeResult> {
        serde_json::from_str(first_json_object(text)?).ok()
    }

    #[test]
    fn r21_trailing_prose_after_the_object_no_longer_fails() {
        // The exact shape observed in the run: valid JSON, blank line, then
        // commentary. `rfind('}')` reached into the prose and serde rejected
        // the span with "trailing characters at line 3 column 1".
        let text = "{\"correct\": true, \"reasoning\": \"The system answer matches.\"}\n\n\
                    Note: the ground truth is ambiguous here (see {2} above).";
        let g = parse(text).expect("must parse");
        assert!(g.correct, "the judge said true; we must not record false");
    }

    #[test]
    fn r21_braces_inside_the_reasoning_string_do_not_end_the_object() {
        let text = "{\"correct\": false, \"reasoning\": \"answer used {placeholder} syntax\"}";
        let g = parse(text).expect("must parse");
        assert!(!g.correct);
        assert!(g.reasoning.unwrap().contains("{placeholder}"));
    }

    #[test]
    fn r21_escaped_quotes_inside_the_string_do_not_end_the_string() {
        let text =
            r#"{"correct": true, "reasoning": "system said \"Rome\" which matches"} trailing }"#;
        let g = parse(text).expect("must parse");
        assert!(g.correct);
        assert!(g.reasoning.unwrap().contains("\"Rome\""));
    }

    #[test]
    fn r21_preamble_before_the_object_is_skipped() {
        let text = "Here is my grade:\n{\"correct\": true, \"reasoning\": \"ok\"}";
        assert!(parse(text).expect("must parse").correct);
    }

    #[test]
    fn r21_nested_objects_return_the_whole_outer_object() {
        let text = "{\"correct\": true, \"reasoning\": \"x\", \"meta\": {\"a\": {\"b\": 1}}} tail";
        let obj = first_json_object(text).unwrap();
        assert!(obj.ends_with("}}}"), "got {obj}");
        assert!(parse(text).unwrap().correct);
    }

    #[test]
    fn r21_no_object_is_still_a_judge_failure() {
        assert!(first_json_object("I cannot grade this.").is_none());
        // Unbalanced (truncated mid-object) must NOT be salvaged into a false
        // verdict — it stays a failure, excluded from the denominator.
        assert!(first_json_object("{\"correct\": true, \"reasoning\": \"cut off").is_none());
    }

    #[test]
    fn r21_old_span_extraction_would_have_failed_these() {
        // Guards the premise: these inputs genuinely break the old approach,
        // so the tests above are not vacuous.
        for text in [
            "{\"correct\": true, \"reasoning\": \"ok\"}\n\nAside: {note}",
            r#"{"correct": true, "reasoning": "said \"Rome\""} trailing }"#,
        ] {
            let (s, e) = (text.find('{').unwrap(), text.rfind('}').unwrap());
            assert!(
                serde_json::from_str::<GradeResult>(&text[s..=e]).is_err(),
                "old extraction unexpectedly succeeded on {text}"
            );
            assert!(
                parse(text).is_some(),
                "new extraction must succeed on {text}"
            );
        }
    }
}
