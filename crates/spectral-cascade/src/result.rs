//! Pipeline result types.

use spectral_ingest::MemoryHit;

/// Machine-readable receipt describing which deterministic ranking signals
/// were active for a cascade result. Per-hit `signal_score` is the final
/// composite score; this receipt makes its configured lineage inspectable.
#[derive(Debug, Clone, PartialEq)]
pub struct RetrievalExplanation {
    pub visibility_boundary: String,
    pub candidate_fetch_multiplier: usize,
    pub applied_signals: Vec<String>,
    pub co_retrieval_weight: f64,
    pub recency_half_life_days: Option<f64>,
    pub episode_diversity_cap: Option<usize>,
}

/// Outcome of a retrieval pipeline run.
pub struct CascadeResult {
    /// Merged hits from retrieval, deduplicated by memory id.
    pub merged_hits: Vec<MemoryHit>,
    /// Estimated tokens consumed (content_len / 4 + 5 per hit).
    pub total_tokens_used: usize,
    /// Highest composite score among returned hits.
    pub max_confidence: f64,
    /// Total LLM tokens consumed during retrieval. Structurally zero:
    /// no `Brain::recall_*()` method makes an LLM call. This field is
    /// the load-bearing receipt for the "no LLM in the recall path"
    /// commitment — it exists so consumers can assert it equals 0.
    pub total_recognition_token_cost: usize,
    /// Configuration receipt for the ranking decisions in this result.
    pub explanation: RetrievalExplanation,
}
