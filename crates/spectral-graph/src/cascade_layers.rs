//! Cascade layer adapters for spectral-graph.

use spectral_cascade::{Layer, LayerId, LayerResult};
use spectral_ingest::MemoryHit;

use crate::brain::{AaakOpts, Brain};

/// L1 AAAK layer: returns foundational facts from the brain's memory store.
pub struct AaakLayer<'b> {
    brain: &'b Brain,
    max_tokens: usize,
}

impl<'b> AaakLayer<'b> {
    pub fn new(brain: &'b Brain, max_tokens: usize) -> Self {
        Self { brain, max_tokens }
    }
}

impl Layer for AaakLayer<'_> {
    fn id(&self) -> LayerId {
        LayerId::L1
    }

    fn query(
        &self,
        _query: &str,
        budget_remaining: usize,
    ) -> Result<LayerResult, Box<dyn std::error::Error + Send + Sync>> {
        let budget = budget_remaining.min(self.max_tokens);
        let result = self.brain.aaak(AaakOpts {
            max_tokens: budget,
            ..AaakOpts::default()
        })?;

        if result.fact_count == 0 {
            return Ok(LayerResult::Skipped {
                reason: "no foundational facts matched".into(),
            });
        }

        // Convert AAAK formatted output into MemoryHit-shaped results.
        // AAAK returns a pre-formatted string, not individual memories.
        // We synthesize a single MemoryHit containing the formatted block.
        let hit = MemoryHit {
            id: "__aaak__".into(),
            key: "__aaak__".into(),
            content: result.formatted,
            wing: None,
            hall: Some("fact".into()),
            signal_score: 1.0,
            visibility: "private".into(),
            hits: result.fact_count,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        };

        // Foundational facts are always grounding — return Sufficient
        // if we found any, so the cascade can stop early for simple
        // factual queries.
        Ok(LayerResult::Sufficient {
            hits: vec![hit],
            tokens_used: result.estimated_tokens,
        })
    }
}

/// L3 Constellation/TACT layer: fingerprint + FTS recall.
pub struct ConstellationLayer<'b> {
    brain: &'b Brain,
    max_tokens: usize,
}

impl<'b> ConstellationLayer<'b> {
    pub fn new(brain: &'b Brain, max_tokens: usize) -> Self {
        Self { brain, max_tokens }
    }
}

impl Layer for ConstellationLayer<'_> {
    fn id(&self) -> LayerId {
        LayerId::L3
    }

    fn query(
        &self,
        query: &str,
        budget_remaining: usize,
    ) -> Result<LayerResult, Box<dyn std::error::Error + Send + Sync>> {
        let result = self
            .brain
            .recall_local(query)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

        if result.memory_hits.is_empty() {
            return Ok(LayerResult::Skipped {
                reason: "no constellation/FTS matches".into(),
            });
        }

        // Estimate tokens: ~4 chars per token
        let budget = budget_remaining.min(self.max_tokens);
        let max_chars = budget * 4;

        let mut chars_used = 0;
        let mut hits = Vec::new();
        for hit in result.memory_hits {
            let hit_chars = hit.content.len() + hit.key.len() + 20; // overhead
            if chars_used + hit_chars > max_chars && !hits.is_empty() {
                break;
            }
            chars_used += hit_chars;
            hits.push(hit);
        }

        let tokens_used = chars_used / 4;

        // Constellation alone may not be sufficient for synthesis
        // questions; return Partial so cascade can fall through to
        // L2 (episode summaries) when it ships.
        Ok(LayerResult::Partial { hits, tokens_used })
    }
}
