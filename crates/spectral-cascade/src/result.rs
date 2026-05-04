//! Cascade result types.

use crate::{LayerId, LayerResult};
use spectral_ingest::MemoryHit;

/// Outcome of a full cascade run.
pub struct CascadeResult {
    /// Per-layer outcomes in execution order.
    pub layer_outcomes: Vec<(LayerId, LayerResult)>,
    /// Merged hits from all layers, deduplicated by memory id.
    pub merged_hits: Vec<MemoryHit>,
    /// Total tokens consumed across all layers.
    pub total_tokens_used: usize,
    /// If the cascade stopped early, which layer caused it.
    pub stopped_at: Option<LayerId>,
    /// Highest confidence reported by any layer.
    pub max_confidence: f64,
    /// Total LLM tokens consumed during recognition across all layers.
    /// Current layers (AAAK, Episode, Constellation) all report 0 since
    /// they use no LLMs — this is the empirical proof artifact for the
    /// zero-LLM recognition claim.
    pub total_recognition_token_cost: usize,
}
