//! Convert LongMemEval conversations into Spectral memories.

use crate::dataset::Question;
use anyhow::Result;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use std::path::Path;

/// How to ingest conversation sessions into the brain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestStrategy {
    /// Each user/assistant message becomes its own memory.
    #[default]
    PerTurn,
    /// Each session is concatenated into a single memory.
    PerSession,
}

/// Create a fresh brain and ingest a question's haystack sessions.
pub fn ingest_question(
    question: &Question,
    brain_dir: &Path,
    strategy: IngestStrategy,
) -> Result<Brain> {
    std::fs::create_dir_all(brain_dir)?;

    // Write minimal ontology
    let ontology_path = brain_dir.join("ontology.toml");
    if !ontology_path.exists() {
        std::fs::write(&ontology_path, "version = 1\n")?;
    }

    let brain = Brain::open(BrainConfig {
        data_dir: brain_dir.to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
    })?;

    let sessions = &question.haystack_sessions;
    let session_ids = &question.haystack_session_ids;
    let dates = &question.haystack_dates;

    for (idx, session) in sessions.iter().enumerate() {
        let session_id = session_ids
            .get(idx)
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        let _date = dates.get(idx).map(|s| s.as_str()).unwrap_or("");

        match strategy {
            IngestStrategy::PerTurn => {
                for (turn_idx, turn) in session.iter().enumerate() {
                    let key = format!("{session_id}:turn:{turn_idx}:{}", turn.role);
                    brain.remember(
                        &key,
                        &turn.content,
                        spectral_core::visibility::Visibility::Private,
                    )?;
                }
            }
            IngestStrategy::PerSession => {
                let content: String = session
                    .iter()
                    .map(|t| format!("{}: {}", t.role, t.content))
                    .collect::<Vec<_>>()
                    .join("\n");
                let key = format!("{session_id}:session");
                brain.remember(
                    &key,
                    &content,
                    spectral_core::visibility::Visibility::Private,
                )?;
            }
        }
    }

    Ok(brain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Turn;

    fn test_question() -> Question {
        Question {
            question_id: "q-test".into(),
            question_type: "abstention".into(),
            question: "What color is the sky?".into(),
            answer: "Blue".into(),
            haystack_sessions: vec![vec![
                Turn {
                    role: "user".into(),
                    content: "The sky is blue today.".into(),
                },
                Turn {
                    role: "assistant".into(),
                    content: "That sounds lovely!".into(),
                },
            ]],
            haystack_session_ids: vec!["s1".into()],
            haystack_dates: vec!["2024-01-15".into()],
        }
    }

    #[test]
    fn per_turn_ingestion_creates_memories() {
        let dir = tempfile::tempdir().unwrap();
        let brain = ingest_question(&test_question(), dir.path(), IngestStrategy::PerTurn).unwrap();
        let result = brain.recall_local("sky blue").unwrap();
        assert!(
            !result.memory_hits.is_empty(),
            "should find ingested memories"
        );
    }

    #[test]
    fn per_session_ingestion_creates_single_memory() {
        let dir = tempfile::tempdir().unwrap();
        let brain =
            ingest_question(&test_question(), dir.path(), IngestStrategy::PerSession).unwrap();
        let result = brain.recall_local("sky blue").unwrap();
        assert!(!result.memory_hits.is_empty());
    }
}
