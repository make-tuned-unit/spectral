//! TACT (Topic-Aware Context Triage) — fingerprint-based memory retrieval.
//!
//! Finds relevant memories from a structured store and formats them for
//! system-prompt injection. No embedding inference required.

pub mod classifier;
pub mod extractor;
pub mod prompts;

// Re-export canonical types from spectral-ingest.
pub use spectral_ingest::{Memory, MemoryHit, MemoryStore};

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// Trait for injecting an LLM implementation (optional — TACT's core
/// pipeline is regex-only and does not call an LLM).
pub trait LlmClient: Send + Sync {
    fn complete(
        &self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + '_>>;
}

/// TACT pipeline configuration.
#[derive(Debug, Clone)]
pub struct TactConfig {
    /// Minimum word count for TACT classification to engage. Queries with
    /// fewer words return `RetrievalMethod::Skipped` with zero results.
    /// Default 1 means single-word queries are classified normally. Set
    /// higher (e.g., 3) if consuming code wants to bypass TACT for
    /// greeting-style short messages.
    pub min_words: usize,
    /// Maximum results to return.
    pub max_results: usize,
    /// Maximum characters in the context bundle (~tokens * 4).
    pub max_context_chars: usize,
    /// Wing detection rules: (regex_pattern, wing_name).
    pub wing_rules: Vec<(String, String)>,
    /// Hall detection rules: (regex_pattern, hall_name).
    pub hall_rules: Vec<(String, String)>,
}

impl Default for TactConfig {
    fn default() -> Self {
        Self {
            min_words: 1,
            max_results: 5,
            max_context_chars: 24000,
            wing_rules: Vec::new(),
            hall_rules: vec![
                (
                    r"decided|chose|switching to|using|will use|agreed|locked in|decision|auth"
                        .into(),
                    "fact".into(),
                ),
                (
                    r"remember|preference|favourit|favorit|likes|prefers".into(),
                    "preference".into(),
                ),
                (
                    r"learned|discovered|found that|realized|breakthrough|roadmap|setup".into(),
                    "discovery".into(),
                ),
                (
                    r"recommend|should|advice|suggest|try using".into(),
                    "advice".into(),
                ),
            ],
        }
    }
}

/// The retrieval method that produced results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RetrievalMethod {
    Fingerprint,
    FingerprintPlusFts,
    WingOnly,
    Fts,
    Skipped,
    Empty,
}

impl std::fmt::Display for RetrievalMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fingerprint => write!(f, "fingerprint"),
            Self::FingerprintPlusFts => write!(f, "fingerprint+fts"),
            Self::WingOnly => write!(f, "wing_only"),
            Self::Fts => write!(f, "fts_fallback"),
            Self::Skipped => write!(f, "skipped"),
            Self::Empty => write!(f, "empty"),
        }
    }
}

/// Full retrieval result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TactResult {
    pub method: RetrievalMethod,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub memories: Vec<MemoryHit>,
    /// Formatted context block for system-prompt injection.
    pub context_block: String,
}

/// Run the full TACT retrieval pipeline.
pub async fn retrieve(
    user_msg: &str,
    config: &TactConfig,
    store: &dyn MemoryStore,
) -> anyhow::Result<TactResult> {
    if user_msg.split_whitespace().count() < config.min_words {
        return Ok(TactResult {
            method: RetrievalMethod::Skipped,
            wing: None,
            hall: None,
            memories: Vec::new(),
            context_block: String::new(),
        });
    }

    let wing = classifier::detect_wing(user_msg, &config.wing_rules);
    let hall = classifier::detect_hall(user_msg, &config.hall_rules);

    let (memories, method) = extractor::search(user_msg, &wing, &hall, config, store).await?;

    if memories.is_empty() {
        return Ok(TactResult {
            method: RetrievalMethod::Empty,
            wing,
            hall,
            memories: Vec::new(),
            context_block: String::new(),
        });
    }

    let bundle = build_context_bundle(&memories, config.max_context_chars);
    let context_block =
        format!("\n--- MEMORY CONTEXT (via TACT) ---\n{bundle}\n--- END MEMORY CONTEXT ---\n");

    Ok(TactResult {
        method,
        wing,
        hall,
        memories,
        context_block,
    })
}

fn build_context_bundle(memories: &[MemoryHit], max_chars: usize) -> String {
    let mut parts = Vec::new();
    let mut char_count = 0;

    for m in memories {
        let wing = m.wing.as_deref().unwrap_or("unknown");
        let hall = m.hall.as_deref().unwrap_or("unknown");
        let entry = format!("[{}/{}] {}: {}", wing, hall, m.key, m.content);

        if char_count + entry.len() > max_chars {
            let remaining = max_chars.saturating_sub(char_count);
            if remaining > 50 {
                parts.push(format!("{}...", &entry[..remaining.min(entry.len())]));
            }
            break;
        }

        char_count += entry.len() + 1;
        parts.push(entry);
    }

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_context_bundle_truncation() {
        let memories = vec![
            MemoryHit {
                id: "1".into(),
                key: "test key".into(),
                content: "a".repeat(200),
                wing: Some("proj".into()),
                hall: Some("fact".into()),
                signal_score: 0.9,
                visibility: "private".into(),
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                hits: 3,
            },
            MemoryHit {
                id: "2".into(),
                key: "another".into(),
                content: "b".repeat(200),
                wing: Some("proj".into()),
                hall: Some("discovery".into()),
                signal_score: 0.7,
                visibility: "private".into(),
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                hits: 1,
            },
        ];

        let result = build_context_bundle(&memories, 100);
        assert!(result.len() <= 110);
    }

    #[test]
    fn test_retrieval_method_display() {
        assert_eq!(RetrievalMethod::Fingerprint.to_string(), "fingerprint");
        assert_eq!(RetrievalMethod::Fts.to_string(), "fts_fallback");
    }
}
