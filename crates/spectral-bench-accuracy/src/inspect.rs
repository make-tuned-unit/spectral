//! Signal-score inspection tools for diagnosing retrieval failures.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

use crate::dataset::Question;
use crate::ingest;
use crate::retrieval::RetrievalConfig;

/// A single scored-memory record for the JSONL dump (Tool 1).
#[derive(Debug, Serialize, Deserialize)]
pub struct ScoreRecord {
    pub question_id: String,
    pub rank: usize,
    pub memory_key: String,
    pub signal_score: f64,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub created_at: Option<String>,
    pub content_preview: String,
    pub predicted_correct: bool,
}

/// A memory with full scoring fields for the inspect output (Tool 2).
#[derive(Debug, Serialize, Deserialize)]
pub struct ScoredMemory {
    pub rank: usize,
    pub memory_key: String,
    pub signal_score: f64,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub created_at: Option<String>,
    pub content: String,
}

/// Full inspection result for a single question (Tool 2).
#[derive(Debug, Serialize, Deserialize)]
pub struct InspectResult {
    pub question_id: String,
    pub question: String,
    pub ground_truth: String,
    pub haystack_memory_count: usize,
    pub retrieved_top_20: Vec<ScoredMemory>,
    pub all_memories: Vec<ScoredMemory>,
}

/// Write score records for one question's retrieved memories to a JSONL writer.
pub fn write_score_records(
    writer: &mut impl Write,
    question_id: &str,
    hits: &[spectral_ingest::MemoryHit],
    predicted_correct: bool,
) -> Result<()> {
    for (i, hit) in hits.iter().enumerate() {
        let record = ScoreRecord {
            question_id: question_id.into(),
            rank: i + 1,
            memory_key: hit.key.clone(),
            signal_score: hit.signal_score,
            wing: hit.wing.clone(),
            hall: hit.hall.clone(),
            created_at: hit.created_at.clone(),
            content_preview: hit.content.chars().take(80).collect(),
            predicted_correct,
        };
        serde_json::to_writer(&mut *writer, &record)?;
        writeln!(writer)?;
    }
    Ok(())
}

/// Run a deep inspection of a single question: ingest, recall top-N,
/// enumerate ALL memories in the brain.
pub fn inspect_question(
    question: &Question,
    work_dir: &Path,
    config: &RetrievalConfig,
) -> Result<InspectResult> {
    let brain_dir = work_dir.join(format!("inspect_{}", question.question_id));
    let brain = ingest::ingest_question(question, &brain_dir, ingest::IngestStrategy::PerTurn)?;

    // Top-N via normal recall
    let recall_result = brain.recall_local(&question.question)?;
    let retrieved_top: Vec<ScoredMemory> = recall_result
        .memory_hits
        .iter()
        .take(config.max_results)
        .enumerate()
        .map(|(i, hit)| ScoredMemory {
            rank: i + 1,
            memory_key: hit.key.clone(),
            signal_score: hit.signal_score,
            wing: hit.wing.clone(),
            hall: hit.hall.clone(),
            created_at: hit.created_at.clone(),
            content: hit.content.clone(),
        })
        .collect();

    // ALL memories from the brain, sorted by signal_score descending
    let all_raw = brain.list_all_memories(10_000)?;
    let haystack_memory_count = all_raw.len();
    let all_memories: Vec<ScoredMemory> = all_raw
        .into_iter()
        .enumerate()
        .map(|(i, mem)| ScoredMemory {
            rank: i + 1,
            memory_key: mem.key,
            signal_score: mem.signal_score,
            wing: mem.wing,
            hall: mem.hall,
            created_at: mem.created_at,
            content: mem.content,
        })
        .collect();

    let _ = std::fs::remove_dir_all(&brain_dir);

    Ok(InspectResult {
        question_id: question.question_id.clone(),
        question: question.question.clone(),
        ground_truth: question.answer_text(),
        haystack_memory_count,
        retrieved_top_20: retrieved_top,
        all_memories,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Turn;

    fn test_question_with_many_turns() -> Question {
        let mut sessions = Vec::new();
        let mut session = Vec::new();
        for i in 0..30 {
            session.push(Turn {
                role: "user".into(),
                content: format!("User message {i} about project milestone {i} progress"),
            });
            session.push(Turn {
                role: "assistant".into(),
                content: format!("Assistant response {i} about project milestone {i}"),
            });
        }
        sessions.push(session);

        Question {
            question_id: "q-inspect-test".into(),
            question_type: "multi-session".into(),
            question: "What project milestones were discussed?".into(),
            answer: serde_json::Value::String("Multiple milestones".into()),
            question_date: Some("2023/05/30 (Tue) 23:40".into()),
            haystack_sessions: sessions,
            haystack_session_ids: vec!["s1".into()],
            haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
        }
    }

    #[test]
    fn inspect_all_memories_includes_more_than_top_20() {
        let dir = tempfile::tempdir().unwrap();
        let q = test_question_with_many_turns();
        let result =
            inspect_question(&q, dir.path(), &RetrievalConfig { max_results: 20 }).unwrap();

        assert!(
            result.all_memories.len() > result.retrieved_top_20.len(),
            "all_memories ({}) should exceed retrieved_top_20 ({})",
            result.all_memories.len(),
            result.retrieved_top_20.len()
        );
        assert_eq!(result.haystack_memory_count, result.all_memories.len());
    }

    #[test]
    fn dump_scores_writes_jsonl() {
        let hits = vec![spectral_ingest::MemoryHit {
            id: "m1".into(),
            key: "s1:turn:0:user".into(),
            content: "Test memory content about project decisions and milestones".into(),
            wing: Some("general".into()),
            hall: Some("fact".into()),
            signal_score: 0.75,
            visibility: "private".into(),
            hits: 1,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: Some("2023-02-15 23:50:00".into()),
            last_reinforced_at: None,
        }];

        let mut buf = Vec::new();
        write_score_records(&mut buf, "q-test", &hits, true).unwrap();
        let line = String::from_utf8(buf).unwrap();
        let record: ScoreRecord = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(record.question_id, "q-test");
        assert_eq!(record.rank, 1);
        assert_eq!(record.memory_key, "s1:turn:0:user");
        assert!((record.signal_score - 0.75).abs() < f64::EPSILON);
        assert!(record.predicted_correct);
    }
}
