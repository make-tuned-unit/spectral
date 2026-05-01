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
            let wing = hit.wing.as_deref().unwrap_or("?");
            let hall = hit.hall.as_deref().unwrap_or("?");
            format!("[{wing}/{hall}] {}: {}", hit.key, hit.content)
        })
        .collect();

    Ok(memories)
}
