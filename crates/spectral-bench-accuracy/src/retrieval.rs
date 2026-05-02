//! Query Spectral and format retrieved context for the actor.

use anyhow::Result;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::Brain;
use std::collections::HashSet;

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

/// Which retrieval path to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalPath {
    /// TACT/FTS recall (default).
    #[default]
    Tact,
    /// Graph traversal: use entity neighborhood to find memories that
    /// FTS misses due to vocabulary mismatch, then fall back to FTS
    /// if the graph produces fewer than 5 results.
    Graph,
}

/// Format a MemoryHit into the standard actor format.
fn format_hit(hit: &spectral_ingest::MemoryHit) -> String {
    let date = hit
        .created_at
        .as_deref()
        .map(|s| s.split('T').next().unwrap_or(s))
        .unwrap_or("unknown-date");
    let wing = hit.wing.as_deref().unwrap_or("?");
    let hall = hit.hall.as_deref().unwrap_or("?");
    format!("[{date}] [{wing}/{hall}] {}: {}", hit.key, hit.content)
}

/// Retrieve memories relevant to a question from a brain.
/// Returns formatted memory strings for the actor.
pub fn retrieve(brain: &Brain, question: &str, config: &RetrievalConfig) -> Result<Vec<String>> {
    let result = brain.recall_local(question)?;

    let memories: Vec<String> = result
        .memory_hits
        .into_iter()
        .take(config.max_results)
        .map(|hit| format_hit(&hit))
        .collect();

    Ok(memories)
}

/// Retrieve memories using graph traversal to bridge vocabulary mismatches.
///
/// 1. Calls `recall_graph` to find entity neighbors of the query.
/// 2. For each entity canonical name, runs an FTS search to find memories
///    mentioning that entity (since `recall_graph` returns entities/triples,
///    not memories directly — this is an architectural gap).
/// 3. Deduplicates across all entity searches.
/// 4. If fewer than 5 memories found via graph, falls back to standard FTS.
pub fn retrieve_graph(
    brain: &Brain,
    question: &str,
    config: &RetrievalConfig,
) -> Result<Vec<String>> {
    let graph_result = brain.recall_graph(question, Visibility::Private)?;

    let mut seen_keys = HashSet::new();
    let mut all_hits = Vec::new();

    // Use each entity's canonical name as a secondary FTS query
    for entity in &graph_result.neighborhood.entities {
        let entity_result = brain.recall_local(&entity.canonical)?;
        for hit in entity_result.memory_hits {
            if seen_keys.insert(hit.key.clone()) {
                all_hits.push(hit);
            }
        }
    }

    // Fall back to standard FTS if graph produced fewer than 5 results
    if all_hits.len() < 5 {
        let fts_result = brain.recall_local(question)?;
        for hit in fts_result.memory_hits {
            if seen_keys.insert(hit.key.clone()) {
                all_hits.push(hit);
            }
        }
    }

    let memories: Vec<String> = all_hits
        .iter()
        .take(config.max_results)
        .map(format_hit)
        .collect();

    Ok(memories)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
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
            tact_config: None,
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

    #[test]
    fn retrieve_graph_runs_without_panic() {
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
            tact_config: None,
        })
        .unwrap();

        brain
            .remember(
                "graph-test",
                "Memories about photography and Sony cameras for testing",
                Visibility::Private,
            )
            .unwrap();

        // With a minimal ontology (no entities defined), graph recall returns
        // no entities. The function should fall back to FTS and still return
        // results without panicking.
        let memories = retrieve_graph(
            &brain,
            "photography Sony cameras testing",
            &RetrievalConfig::default(),
        )
        .unwrap();
        assert!(
            !memories.is_empty(),
            "graph retrieval should fall back to FTS when no entities found"
        );
    }
}
