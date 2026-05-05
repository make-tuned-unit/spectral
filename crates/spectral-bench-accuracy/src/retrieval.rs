//! Query Spectral and format retrieved context for the actor.

use anyhow::Result;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, RecallTopKConfig};
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
    /// Top-K FTS with re-ranking (default — Phase 1 improvement).
    #[default]
    TopkFts,
    /// TACT/FTS recall (legacy).
    Tact,
    /// Graph traversal.
    Graph,
    /// Cascade (L1→L2→L3).
    Cascade,
}

/// Format a MemoryHit into the standard actor format.
pub fn format_hit(hit: &spectral_ingest::MemoryHit) -> String {
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

/// Retrieve via top-K FTS with additive re-ranking. No LLM cost.
///
/// Configurable via env vars for ablation:
/// - `SPECTRAL_DISABLE_SIGNAL_SCORE=1` — disable signal weighting
/// - `SPECTRAL_DISABLE_RECENCY=1` — disable recency weighting
/// - `SPECTRAL_DISABLE_ENTITY_RESOLUTION=1` — disable entity clustering
/// - `SPECTRAL_DISABLE_CONTEXT_DEDUP=1` — disable context chain dedup
pub fn retrieve_topk_fts(
    brain: &Brain,
    question: &str,
    config: &RetrievalConfig,
) -> Result<Vec<String>> {
    let topk_config = RecallTopKConfig {
        k: config.max_results.max(40),
        apply_signal_score_weighting: std::env::var("SPECTRAL_DISABLE_SIGNAL_SCORE").is_err(),
        apply_recency_weighting: std::env::var("SPECTRAL_DISABLE_RECENCY").is_err(),
        apply_entity_resolution: std::env::var("SPECTRAL_DISABLE_ENTITY_RESOLUTION").is_err(),
        apply_context_dedup: std::env::var("SPECTRAL_DISABLE_CONTEXT_DEDUP").is_err(),
        ..RecallTopKConfig::default()
    };

    let hits = brain.recall_topk_fts(question, &topk_config, Visibility::Private)?;

    let memories: Vec<String> = hits
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

/// Telemetry from a cascade retrieval run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CascadeTelemetry {
    pub stopped_at: Option<String>,
    pub max_confidence: f64,
    pub total_tokens_used: usize,
    /// Total LLM tokens consumed during recognition across all layers.
    /// Expected to be 0 for all current layers (empirical proof artifact).
    pub total_recognition_token_cost: usize,
    pub layer_outcomes: Vec<(String, String)>,
}

/// Retrieve memories via the cascade (L1→L2→L3).
pub fn retrieve_cascade(
    brain: &Brain,
    question: &str,
    config: &RetrievalConfig,
) -> Result<(Vec<String>, CascadeTelemetry)> {
    let cascade_config = spectral_cascade::orchestrator::CascadeConfig::default();
    // Bench has no ambient signal (LongMemEval is synthetic), so empty context is correct.
    let context = spectral_cascade::RecognitionContext::empty();
    let result = brain.recall_cascade(question, &context, &cascade_config)?;

    let formatted: Vec<String> = result
        .merged_hits
        .iter()
        .take(config.max_results)
        .map(format_hit)
        .collect();

    let telemetry = CascadeTelemetry {
        stopped_at: result.stopped_at.map(|id| id.to_string()),
        max_confidence: result.max_confidence,
        total_tokens_used: result.total_tokens_used,
        total_recognition_token_cost: result.total_recognition_token_cost,
        layer_outcomes: result
            .layer_outcomes
            .iter()
            .map(|(id, r)| {
                let status = match r {
                    spectral_cascade::LayerResult::Sufficient { confidence, .. } => {
                        format!("sufficient(confidence={confidence:.2})")
                    }
                    spectral_cascade::LayerResult::Partial { confidence, .. } => {
                        format!("partial(confidence={confidence:.2})")
                    }
                    spectral_cascade::LayerResult::Skipped { reason, .. } => {
                        format!("skipped({reason})")
                    }
                };
                (id.to_string(), status)
            })
            .collect(),
    };

    Ok((formatted, telemetry))
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
