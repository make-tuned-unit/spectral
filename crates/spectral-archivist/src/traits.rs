//! Pluggable traits for LLM-mediated maintenance passes (Phase 2).
//!
//! Default implementations are no-ops. Consumers can provide their own
//! LLM clients to enable consolidation and index generation.

/// Pluggable consolidation. Takes two memory contents, returns a merged summary.
pub trait Consolidator: Send + Sync {
    fn consolidate(&self, content_a: &str, content_b: &str) -> anyhow::Result<Option<String>>;
}

/// No-op consolidator that always returns None. Default for Phase 1.
pub struct NoOpConsolidator;

impl Consolidator for NoOpConsolidator {
    fn consolidate(&self, _a: &str, _b: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

/// Pluggable wing index generation. Takes a wing name and top memories,
/// returns a one-paragraph summary.
pub trait Indexer: Send + Sync {
    fn generate_index(
        &self,
        wing: &str,
        memories: &[(String, String, Option<String>)], // (key, content, hall)
    ) -> anyhow::Result<Option<String>>;
}

/// No-op indexer that always returns None. Default for Phase 1.
pub struct NoOpIndexer;

impl Indexer for NoOpIndexer {
    fn generate_index(
        &self,
        _wing: &str,
        _memories: &[(String, String, Option<String>)],
    ) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}
