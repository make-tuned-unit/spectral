//! Convert LongMemEval conversations into Spectral memories.

use crate::dataset::Question;
use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, Utc};
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use spectral_tact::TactConfig;
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

/// The memory key an ingested turn receives.
///
/// THE authority for the bench key format. The oracle's evidence scoring
/// builds its expected-key set through this same function, so the two can
/// never drift; if they did, evidence recall would silently read 0% and look
/// like a catastrophic retrieval regression. Frozen by
/// `memory_key_format_is_frozen` — changing the output invalidates every
/// archived bench brain and every archived `retrieved_keys` list.
pub fn memory_key(
    strategy: IngestStrategy,
    session_id: &str,
    turn_idx: usize,
    role: &str,
) -> String {
    match strategy {
        IngestStrategy::PerTurn => format!("{session_id}:turn:{turn_idx}:{role}"),
        IngestStrategy::PerSession => format!("{session_id}:session"),
    }
}

/// Parse a LongMemEval date string like `"2023/02/15 (Wed) 23:50"` into `DateTime<Utc>`.
fn parse_haystack_date(s: &str) -> Option<DateTime<Utc>> {
    match NaiveDateTime::parse_from_str(s, "%Y/%m/%d (%a) %H:%M") {
        Ok(ndt) => Some(ndt.and_utc()),
        Err(e) => {
            eprintln!("warn: failed to parse haystack date {s:?}: {e}");
            None
        }
    }
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

    let mut brain_config = BrainConfig {
        data_dir: brain_dir.to_path_buf(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        // Backlog item 22: spectrogram attribution. Env-gated like the
        // SPECTRAL_DISABLE_* ablation flags; default off (published config).
        enable_spectrogram: std::env::var("SPECTRAL_BENCH_SPECTROGRAM").is_ok(),
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        fts_tokenizer: None,
        read_only: false,
        activity_wing: "activity".into(),
        redaction_policy: None,
        // Override TACT to return up to 20 results — multi-session questions
        // need memories from 3+ sessions, which top-K=5 physically cannot provide.
        tact_config: Some(TactConfig {
            max_results: 20,
            ..TactConfig::default()
        }),
        ..Default::default()
    };
    crate::env_levers::apply_env_levers(&mut brain_config);
    let brain = Brain::open(brain_config)?;

    let sessions = &question.haystack_sessions;
    let session_ids = &question.haystack_session_ids;
    let dates = &question.haystack_dates;

    for (idx, session) in sessions.iter().enumerate() {
        let session_id = session_ids
            .get(idx)
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        let date = dates.get(idx).map(|s| s.as_str()).unwrap_or("");
        let created_at = parse_haystack_date(date);

        match strategy {
            IngestStrategy::PerTurn => {
                for (turn_idx, turn) in session.iter().enumerate() {
                    let key = memory_key(strategy, session_id, turn_idx, &turn.role);
                    brain.remember_with(
                        &key,
                        &turn.content,
                        RememberOpts {
                            created_at,
                            episode_id: Some(session_id.to_string()),
                            visibility: Visibility::Private,
                            ..Default::default()
                        },
                    )?;
                }
            }
            IngestStrategy::PerSession => {
                let content: String = session
                    .iter()
                    .map(|t| format!("{}: {}", t.role, t.content))
                    .collect::<Vec<_>>()
                    .join("\n");
                let key = memory_key(strategy, session_id, 0, "");
                brain.remember_with(
                    &key,
                    &content,
                    RememberOpts {
                        created_at,
                        episode_id: Some(session_id.to_string()),
                        visibility: Visibility::Private,
                        ..Default::default()
                    },
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
            question_type: "multi-session".into(),
            question: "What color is the sky?".into(),
            answer: serde_json::Value::String("Blue".into()),
            question_date: Some("2023/05/30 (Tue) 23:40".into()),
            haystack_sessions: vec![vec![
                Turn {
                    role: "user".into(),
                    content: "The sky is blue today.".into(),
                    has_answer: None,
                },
                Turn {
                    role: "assistant".into(),
                    content: "That sounds lovely!".into(),
                    has_answer: None,
                },
            ]],
            haystack_session_ids: vec!["s1".into()],
            haystack_dates: vec!["2023/02/15 (Wed) 23:50".into()],
        }
    }

    #[test]
    fn memory_key_format_is_frozen() {
        // These literals ARE the archive: every bench brain on disk and every
        // `retrieved_keys` list in every archived oracle JSONL uses them.
        // Changing either string invalidates all of it.
        assert_eq!(
            memory_key(IngestStrategy::PerTurn, "answer_abc_1", 7, "assistant"),
            "answer_abc_1:turn:7:assistant"
        );
        assert_eq!(
            memory_key(IngestStrategy::PerSession, "answer_abc_1", 7, "assistant"),
            "answer_abc_1:session"
        );
    }

    #[test]
    fn ingested_keys_match_memory_key_helper() {
        let dir = tempfile::tempdir().unwrap();
        let q = test_question();
        let brain = ingest_question(&q, dir.path(), IngestStrategy::PerTurn).unwrap();
        let stored: std::collections::HashSet<String> = brain
            .list_all_memories(1000)
            .unwrap()
            .into_iter()
            .map(|m| m.key)
            .collect();
        for (turn_idx, turn) in q.haystack_sessions[0].iter().enumerate() {
            let key = memory_key(IngestStrategy::PerTurn, "s1", turn_idx, &turn.role);
            assert!(
                stored.contains(&key),
                "ingest wrote a key the helper does not reproduce: {key}; stored {stored:?}"
            );
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

    #[test]
    fn ingest_with_valid_haystack_date_uses_parsed_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let brain = ingest_question(&test_question(), dir.path(), IngestStrategy::PerTurn).unwrap();
        let result = brain.recall_local("sky blue").unwrap();
        assert!(!result.memory_hits.is_empty());
        let stored = result.memory_hits[0].created_at.as_deref().unwrap();
        assert!(
            stored.starts_with("2023-02-15"),
            "expected created_at starting with 2023-02-15, got {stored}"
        );
    }

    #[test]
    fn ingest_with_malformed_haystack_date_falls_back_to_now() {
        let before = Utc::now();
        let q = Question {
            question_id: "q-bad-date".into(),
            question_type: "multi-session".into(),
            question: "test?".into(),
            answer: serde_json::Value::String("test".into()),
            question_date: None,
            haystack_sessions: vec![vec![Turn {
                role: "user".into(),
                content: "Malformed date memory about project status".into(),
                has_answer: None,
            }]],
            haystack_session_ids: vec!["s-bad".into()],
            haystack_dates: vec!["not a date".into()],
        };
        let dir = tempfile::tempdir().unwrap();
        let brain = ingest_question(&q, dir.path(), IngestStrategy::PerTurn).unwrap();
        let result = brain
            .recall_local("malformed date memory project status")
            .unwrap();
        assert!(!result.memory_hits.is_empty());
        let stored = result.memory_hits[0].created_at.as_deref().unwrap();
        let parsed =
            chrono::NaiveDateTime::parse_from_str(stored, "%Y-%m-%d %H:%M:%S").expect("parse");
        let diff = (parsed.and_utc() - before).num_seconds().abs();
        assert!(
            diff < 5,
            "expected created_at within 5s of now, got {diff}s difference"
        );
    }

    #[test]
    fn ingest_per_session_also_uses_haystack_date() {
        let dir = tempfile::tempdir().unwrap();
        let brain =
            ingest_question(&test_question(), dir.path(), IngestStrategy::PerSession).unwrap();
        let result = brain.recall_local("sky blue").unwrap();
        assert!(!result.memory_hits.is_empty());
        let stored = result.memory_hits[0].created_at.as_deref().unwrap();
        assert!(
            stored.starts_with("2023-02-15"),
            "expected created_at starting with 2023-02-15, got {stored}"
        );
    }

    #[test]
    fn episode_id_threaded_through_ingest() {
        let dir = tempfile::tempdir().unwrap();
        let q = Question {
            question_id: "q-ep".into(),
            question_type: "multi-session".into(),
            question: "test?".into(),
            answer: serde_json::Value::String("test".into()),
            question_date: None,
            haystack_sessions: vec![
                vec![
                    Turn {
                        role: "user".into(),
                        content: "Session one content about project alpha".into(),
                        has_answer: None,
                    },
                    Turn {
                        role: "assistant".into(),
                        content: "I see, project alpha looks good".into(),
                        has_answer: None,
                    },
                ],
                vec![
                    Turn {
                        role: "user".into(),
                        content: "Session two content about project beta".into(),
                        has_answer: None,
                    },
                    Turn {
                        role: "assistant".into(),
                        content: "Project beta is interesting".into(),
                        has_answer: None,
                    },
                ],
            ],
            haystack_session_ids: vec!["sess-alpha".into(), "sess-beta".into()],
            haystack_dates: vec![
                "2023/02/15 (Wed) 23:50".into(),
                "2023/02/16 (Thu) 10:00".into(),
            ],
        };
        let brain = ingest_question(&q, dir.path(), IngestStrategy::PerTurn).unwrap();

        let episodes = brain.list_episodes(None, 100).unwrap();
        assert_eq!(
            episodes.len(),
            2,
            "should create 2 episodes from 2 sessions"
        );

        let ids: Vec<&str> = episodes.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"sess-alpha"));
        assert!(ids.contains(&"sess-beta"));
    }

    #[test]
    fn cascade_telemetry_populated_when_use_cascade() {
        use crate::retrieval::{retrieve_cascade, RetrievalConfig};

        let dir = tempfile::tempdir().unwrap();
        let brain = ingest_question(&test_question(), dir.path(), IngestStrategy::PerTurn).unwrap();

        let (memories, _, telemetry) =
            retrieve_cascade(&brain, "sky blue", &RetrievalConfig::default(), None).unwrap();
        assert!(!memories.is_empty());

        // Verify telemetry serializes to JSON
        let json = serde_json::to_string(&telemetry).unwrap();
        assert!(json.contains("max_confidence"));
        assert!(json.contains("total_tokens_used"));
    }
}
