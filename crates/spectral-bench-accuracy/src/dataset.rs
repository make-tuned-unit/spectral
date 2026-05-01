//! LongMemEval_S dataset loading and types.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single question from the LongMemEval_S dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub question_id: String,
    pub question_type: String,
    pub question: String,
    pub answer: serde_json::Value,
    /// The date context for this question (e.g. "2023/05/30 (Tue) 23:40").
    #[serde(default)]
    pub question_date: Option<String>,
    /// Sessions of conversation turns forming the haystack.
    #[serde(default)]
    pub haystack_sessions: Vec<Vec<Turn>>,
    #[serde(default)]
    pub haystack_session_ids: Vec<String>,
    #[serde(default)]
    pub haystack_dates: Vec<String>,
}

impl Question {
    /// Return the answer as a plain string.
    pub fn answer_text(&self) -> String {
        match &self.answer {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}

/// A single conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

/// LongMemEval_S question category.
///
/// Variants match the actual `question_type` values in the LongMemEval_S dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    MultiSession,
    TemporalReasoning,
    KnowledgeUpdate,
    SingleSessionUser,
    SingleSessionAssistant,
    SingleSessionPreference,
}

impl Category {
    /// Parse a `question_type` string from the dataset into a Category.
    ///
    /// Returns `Err` for unrecognized values rather than silently falling back.
    pub fn from_question_type(s: &str) -> Result<Self> {
        match s {
            "multi-session" => Ok(Self::MultiSession),
            "temporal-reasoning" => Ok(Self::TemporalReasoning),
            "knowledge-update" => Ok(Self::KnowledgeUpdate),
            "single-session-user" => Ok(Self::SingleSessionUser),
            "single-session-assistant" => Ok(Self::SingleSessionAssistant),
            "single-session-preference" => Ok(Self::SingleSessionPreference),
            other => Err(anyhow::anyhow!("unknown question_type: {other}")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MultiSession => "multi-session",
            Self::TemporalReasoning => "temporal-reasoning",
            Self::KnowledgeUpdate => "knowledge-update",
            Self::SingleSessionUser => "single-session-user",
            Self::SingleSessionAssistant => "single-session-assistant",
            Self::SingleSessionPreference => "single-session-preference",
        }
    }

    pub fn all() -> &'static [Category] {
        &[
            Self::MultiSession,
            Self::TemporalReasoning,
            Self::KnowledgeUpdate,
            Self::SingleSessionUser,
            Self::SingleSessionAssistant,
            Self::SingleSessionPreference,
        ]
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Load the LongMemEval_S dataset from a JSON file.
///
/// The file is a top-level JSON array of [`Question`] objects.
pub fn load_dataset(path: &Path) -> Result<Vec<Question>> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read dataset from {}", path.display()))?;
    let questions: Vec<Question> =
        serde_json::from_str(&contents).with_context(|| "failed to parse LongMemEval_S JSON")?;
    Ok(questions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_question_type_maps_all_real_dataset_values() {
        let cases = [
            ("multi-session", Category::MultiSession),
            ("temporal-reasoning", Category::TemporalReasoning),
            ("knowledge-update", Category::KnowledgeUpdate),
            ("single-session-user", Category::SingleSessionUser),
            ("single-session-assistant", Category::SingleSessionAssistant),
            (
                "single-session-preference",
                Category::SingleSessionPreference,
            ),
        ];
        for (input, expected) in &cases {
            let got = Category::from_question_type(input).unwrap();
            assert_eq!(got, *expected, "mismatch for {input:?}");
        }
    }

    #[test]
    fn from_question_type_errors_on_unknown() {
        let result = Category::from_question_type("abstention");
        assert!(result.is_err());
        let result = Category::from_question_type("information_extraction");
        assert!(result.is_err());
    }

    #[test]
    fn parse_minimal_dataset() {
        let json = r#"[{"question_id": "q1", "question_type": "multi-session", "question": "What color?", "answer": "Blue"}]"#;
        let qs: Vec<Question> = serde_json::from_str(json).unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].question_id, "q1");
    }

    #[test]
    fn parse_dataset_with_string_answer() {
        let json = r#"[{"question_id": "q1", "question_type": "multi-session", "question": "Q?", "answer": "Blue"}]"#;
        let qs: Vec<Question> = serde_json::from_str(json).unwrap();
        assert_eq!(qs[0].answer_text(), "Blue");
    }

    #[test]
    fn parse_dataset_with_integer_answer() {
        let json = r#"[{"question_id": "q1", "question_type": "multi-session", "question": "Q?", "answer": 3}]"#;
        let qs: Vec<Question> = serde_json::from_str(json).unwrap();
        assert_eq!(qs[0].answer_text(), "3");
    }

    #[test]
    fn parse_dataset_with_array_answer() {
        let json = r#"[{"question_id": "q1", "question_type": "multi-session", "question": "Q?", "answer": ["a", "b"]}]"#;
        let qs: Vec<Question> = serde_json::from_str(json).unwrap();
        let text = qs[0].answer_text();
        assert!(
            text.contains("a") && text.contains("b"),
            "expected array stringified, got {text}"
        );
    }

    #[test]
    fn parse_dataset_with_question_date() {
        let json = r#"[{"question_id": "q1", "question_type": "multi-session", "question": "Q?", "answer": "A", "question_date": "2023/05/30 (Tue) 23:40"}]"#;
        let qs: Vec<Question> = serde_json::from_str(json).unwrap();
        assert_eq!(
            qs[0].question_date.as_deref(),
            Some("2023/05/30 (Tue) 23:40")
        );
    }
}
