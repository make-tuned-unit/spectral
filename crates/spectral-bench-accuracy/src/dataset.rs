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
    pub answer: String,
    /// Sessions of conversation turns forming the haystack.
    #[serde(default)]
    pub haystack_sessions: Vec<Vec<Turn>>,
    #[serde(default)]
    pub haystack_session_ids: Vec<String>,
    #[serde(default)]
    pub haystack_dates: Vec<String>,
}

/// A single conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

/// The full LongMemEval_S dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dataset {
    pub questions: Vec<Question>,
}

/// LongMemEval_S question category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    InformationExtraction,
    MultiSessionReasoning,
    TemporalReasoning,
    KnowledgeUpdate,
    Abstention,
    SingleSessionPreference,
}

impl Category {
    pub fn from_question_type(s: &str) -> Self {
        match s
            .to_lowercase()
            .replace([' ', '-'], "_")
            .trim()
            .to_string()
            .as_str()
        {
            "information_extraction" => Self::InformationExtraction,
            "multi_session_reasoning" => Self::MultiSessionReasoning,
            "temporal_reasoning" => Self::TemporalReasoning,
            "knowledge_update" | "knowledge_updates" => Self::KnowledgeUpdate,
            "abstention" => Self::Abstention,
            "single_session_preference" | "single_session" => Self::SingleSessionPreference,
            _ => Self::InformationExtraction, // fallback
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InformationExtraction => "information_extraction",
            Self::MultiSessionReasoning => "multi_session_reasoning",
            Self::TemporalReasoning => "temporal_reasoning",
            Self::KnowledgeUpdate => "knowledge_update",
            Self::Abstention => "abstention",
            Self::SingleSessionPreference => "single_session_preference",
        }
    }

    pub fn all() -> &'static [Category] {
        &[
            Self::InformationExtraction,
            Self::MultiSessionReasoning,
            Self::TemporalReasoning,
            Self::KnowledgeUpdate,
            Self::Abstention,
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
pub fn load_dataset(path: &Path) -> Result<Dataset> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read dataset from {}", path.display()))?;
    let dataset: Dataset =
        serde_json::from_str(&contents).with_context(|| "failed to parse LongMemEval_S JSON")?;
    Ok(dataset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_from_question_type() {
        assert_eq!(
            Category::from_question_type("information_extraction"),
            Category::InformationExtraction
        );
        assert_eq!(
            Category::from_question_type("multi-session-reasoning"),
            Category::MultiSessionReasoning
        );
        assert_eq!(
            Category::from_question_type("Single Session Preference"),
            Category::SingleSessionPreference
        );
    }

    #[test]
    fn parse_minimal_dataset() {
        let json = r#"{"questions": [{"question_id": "q1", "question_type": "abstention", "question": "What color?", "answer": "Blue"}]}"#;
        let ds: Dataset = serde_json::from_str(json).unwrap();
        assert_eq!(ds.questions.len(), 1);
        assert_eq!(ds.questions[0].question_id, "q1");
    }
}
