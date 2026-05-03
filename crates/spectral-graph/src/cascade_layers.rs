//! Cascade layer adapters for spectral-graph.

use std::collections::HashMap;

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
        // AaakLayer applies stricter calibration than AaakOpts::default()
        // because returning Sufficient short-circuits L2 and L3 in the
        // cascade. False positives (firing on incidentally-classified
        // conversational data) are more costly than false negatives.
        //
        // 0.85 is the score for a fact-classified memory with one boost
        // keyword (decided/chose/switched): fact base 0.7 + decision
        // boost 0.15 = 0.85. This separates genuine single-fact
        // statements from conversational text that only matches the fact
        // regex without boost keywords (which scores 0.7).
        //
        // Hall restricted to "fact" only: preference/decision/rule halls
        // fire too readily on conversational patterns.
        let result = self.brain.aaak(AaakOpts {
            max_tokens: budget,
            min_signal_score: 0.85,
            include_halls: vec!["fact".into()],
            ..AaakOpts::default()
        })?;

        if result.fact_count == 0 {
            return Ok(LayerResult::Skipped {
                reason: "no foundational facts matched".into(),
                confidence: 0.0,
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
            episode_id: None,
        };

        // Foundational facts are always grounding — return Sufficient
        // if we found any, so the cascade can stop early for simple
        // factual queries.
        Ok(LayerResult::Sufficient {
            hits: vec![hit],
            tokens_used: result.estimated_tokens,
            confidence: 0.95,
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
                confidence: 0.0,
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

        // Confidence proportional to top hit's signal score, capped
        // below threshold so L3 alone doesn't trip early stopping.
        let confidence = hits
            .first()
            .map(|h| h.signal_score.min(0.85))
            .unwrap_or(0.0);

        // Constellation alone may not be sufficient for synthesis
        // questions; return Partial so cascade can fall through to
        // L2 (episode summaries) when it ships.
        Ok(LayerResult::Partial {
            hits,
            tokens_used,
            confidence,
        })
    }
}

/// L2 Episode layer: retrieves memories grouped by episode_id.
///
/// Runs FTS, groups results by episode_id, scores episodes by
/// sum of constituent signal scores, returns the top episode's
/// full memory set. Returns Sufficient when one episode clearly
/// dominates.
pub struct EpisodeLayer<'b> {
    brain: &'b Brain,
    max_tokens: usize,
}

impl<'b> EpisodeLayer<'b> {
    pub fn new(brain: &'b Brain, max_tokens: usize) -> Self {
        Self { brain, max_tokens }
    }
}

impl Layer for EpisodeLayer<'_> {
    fn id(&self) -> LayerId {
        LayerId::L2
    }

    fn query(
        &self,
        query: &str,
        budget_remaining: usize,
    ) -> Result<LayerResult, Box<dyn std::error::Error + Send + Sync>> {
        let recall = self
            .brain
            .recall_local(query)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

        // Group FTS hits by episode_id, sum signal scores per episode
        let mut episode_scores: HashMap<String, f64> = HashMap::new();
        for hit in &recall.memory_hits {
            if let Some(ref ep_id) = hit.episode_id {
                *episode_scores.entry(ep_id.clone()).or_default() += hit.signal_score;
            }
        }

        if episode_scores.is_empty() {
            return Ok(LayerResult::Skipped {
                reason: "no memories with episode_id matched".into(),
                confidence: 0.0,
            });
        }

        // Sort by score descending
        let mut sorted: Vec<_> = episode_scores.into_iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (top_id, top_score) = sorted[0].clone();
        let second_score = sorted.get(1).map(|x| x.1).unwrap_or(0.0);

        // Fetch all memories for the top episode
        let episode_memories = self
            .brain
            .list_memories_by_episode(&top_id)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

        // Convert to MemoryHits, truncate by token budget
        let budget = budget_remaining.min(self.max_tokens);
        let max_chars = budget * 4;
        let mut chars_used = 0;
        let mut hits = Vec::new();

        for mem in &episode_memories {
            let hit_chars = mem.content.len() + mem.key.len() + 20;
            if chars_used + hit_chars > max_chars && !hits.is_empty() {
                break;
            }
            chars_used += hit_chars;
            hits.push(MemoryHit {
                id: mem.id.clone(),
                key: mem.key.clone(),
                content: mem.content.clone(),
                wing: mem.wing.clone(),
                hall: mem.hall.clone(),
                signal_score: mem.signal_score,
                visibility: mem.visibility.clone(),
                hits: 0,
                source: mem.source.clone(),
                device_id: mem.device_id,
                confidence: mem.confidence,
                created_at: mem.created_at.clone(),
                last_reinforced_at: mem.last_reinforced_at.clone(),
                episode_id: mem.episode_id.clone(),
            });
        }

        let tokens_used = chars_used / 4;

        // Confidence based on relevance ratio
        let ratio = if second_score > 0.0 {
            top_score / second_score
        } else {
            f64::INFINITY
        };

        if ratio >= 1.5 {
            let confidence = (ratio / 3.0).min(0.92);
            Ok(LayerResult::Sufficient {
                hits,
                tokens_used,
                confidence,
            })
        } else {
            Ok(LayerResult::Partial {
                hits,
                tokens_used,
                confidence: 0.5,
            })
        }
    }
}
