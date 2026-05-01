//! Query Spectral and format retrieved context for the actor.

use anyhow::Result;
use spectral_graph::brain::Brain;

/// Configuration for retrieval.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetrievalConfig {
    /// Maximum number of memories to retrieve per question.
    pub max_results: usize,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self { max_results: 20 }
    }
}

/// Retrieve memories relevant to a question from a brain.
/// Returns formatted memory strings for the actor.
pub fn retrieve(brain: &Brain, question: &str, config: &RetrievalConfig) -> Result<Vec<String>> {
    let result = brain.recall_local(question)?;

    let memories: Vec<String> = result
        .memory_hits
        .into_iter()
        .take(config.max_results)
        .map(|hit| {
            let date = hit
                .created_at
                .as_deref()
                .map(|s| s.split('T').next().unwrap_or(s))
                .unwrap_or("unknown-date");
            let wing = hit.wing.as_deref().unwrap_or("?");
            let hall = hit.hall.as_deref().unwrap_or("?");
            format!("[{date}] [{wing}/{hall}] {}: {}", hit.key, hit.content)
        })
        .collect();

    Ok(memories)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use spectral_core::visibility::Visibility;
    use spectral_graph::brain::{BrainConfig, EntityPolicy, RememberOpts};

    #[test]
    fn retrieve_includes_created_at_in_format() {
        let dir = tempfile::tempdir().unwrap();
        let ontology_path = dir.path().join("ontology.toml");
        std::fs::write(&ontology_path, "version = 1\n").unwrap();

        let brain = Brain::open(BrainConfig {
            data_dir: dir.path().to_path_buf(),
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
        })
        .unwrap();

        let ts = Utc.with_ymd_and_hms(2023, 6, 15, 12, 0, 0).unwrap();
        brain
            .remember_with(
                "test-date-key",
                "Memory about the project launch date for retrieval test",
                RememberOpts {
                    created_at: Some(ts),
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();

        let memories = retrieve(
            &brain,
            "project launch date retrieval test",
            &RetrievalConfig::default(),
        )
        .unwrap();
        assert!(!memories.is_empty());
        assert!(
            memories[0].contains("2023-06-15"),
            "expected date prefix in formatted memory, got: {}",
            memories[0]
        );
    }
}
