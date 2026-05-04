//! Cascade layer adapters for spectral-graph.

use std::collections::{HashMap, HashSet};

use spectral_cascade::{Layer, LayerId, LayerResult, RecognitionContext};
use spectral_ingest::MemoryHit;

use crate::brain::{AaakOpts, Brain};

// ── Ambient boost helpers ───────────────────────────────────────────

/// Compute ambient boost for a memory hit based on wing alignment and recency.
/// Returns a value in [0.5, 2.0]. Identity (1.0) when context is empty.
fn ambient_boost_for_hit(hit: &MemoryHit, context: &RecognitionContext) -> f64 {
    if context.is_empty() {
        return 1.0;
    }

    let mut boost: f64 = 1.0;

    let activity_wings: HashSet<&str> = context
        .recent_activity
        .iter()
        .filter_map(|e| e.wing.as_deref())
        .collect();

    let hit_wing = hit.wing.as_deref();

    // Wing alignment
    let wing_match = hit_wing.is_some()
        && (context.focus_wing.as_deref() == hit_wing
            || hit_wing.is_some_and(|w| activity_wings.contains(w)));

    if wing_match {
        boost *= 1.5;
    }

    // Recency boost based on created_at vs context.now
    if let Some(ref created_str) = hit.created_at {
        if let Ok(created) = chrono::NaiveDateTime::parse_from_str(created_str, "%Y-%m-%d %H:%M:%S")
        {
            let created_utc = created.and_utc();
            let age_minutes = (context.now - created_utc).num_minutes();
            if (0..60).contains(&age_minutes) {
                boost *= 1.3;
            } else if (60..1440).contains(&age_minutes) {
                boost *= 1.1;
            }
        }
    }

    // Wing mismatch with strong context: downrank
    let has_strong_context = context.focus_wing.is_some() || !context.recent_activity.is_empty();
    if has_strong_context && !wing_match {
        boost *= 0.7;
    }

    boost.clamp(0.5, 2.0)
}

/// Compute ambient boost for an episode based on wing alignment and recency.
/// Returns a value in [0.5, 2.0]. Identity (1.0) when context is empty.
fn ambient_boost_for_episode(
    episode_wing: &str,
    episode_started_at: Option<&str>,
    context: &RecognitionContext,
) -> f64 {
    if context.is_empty() {
        return 1.0;
    }

    let mut boost: f64 = 1.0;

    let activity_wings: HashSet<&str> = context
        .recent_activity
        .iter()
        .filter_map(|e| e.wing.as_deref())
        .collect();

    let wing_match = context.focus_wing.as_deref() == Some(episode_wing)
        || activity_wings.contains(episode_wing);

    if wing_match {
        boost *= 1.5;
    }

    // Recency boost
    if let Some(started_str) = episode_started_at {
        if let Ok(started) = chrono::NaiveDateTime::parse_from_str(started_str, "%Y-%m-%d %H:%M:%S")
        {
            let started_utc = started.and_utc();
            let age_minutes = (context.now - started_utc).num_minutes();
            if (0..60).contains(&age_minutes) {
                boost *= 1.3;
            } else if (60..1440).contains(&age_minutes) {
                boost *= 1.1;
            }
        }
    }

    // Wing mismatch with strong context: downrank
    let has_strong_context = context.focus_wing.is_some() || !context.recent_activity.is_empty();
    if has_strong_context && !wing_match {
        boost *= 0.7;
    }

    boost.clamp(0.5, 2.0)
}

// ── L1: AaakLayer ──────────────────────────────────────────────────

/// L1 AAAK layer: returns foundational facts from the brain's memory store.
///
/// Context-driven: fires only when ambient context provides wing signals.
/// With empty context (bench, ad-hoc queries), returns Skipped so the
/// cascade falls through to L2/L3.
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
        context: &RecognitionContext,
    ) -> Result<LayerResult, Box<dyn std::error::Error + Send + Sync>> {
        // Determine relevant wings from context
        let relevant_wings: HashSet<String> = {
            let mut wings = HashSet::new();
            if let Some(ref focus) = context.focus_wing {
                wings.insert(focus.clone());
            }
            for episode in &context.recent_activity {
                if let Some(ref wing) = episode.wing {
                    wings.insert(wing.clone());
                }
            }
            wings
        };

        // No ambient signal — AAAK has no basis to fire
        if relevant_wings.is_empty() {
            return Ok(LayerResult::Skipped {
                reason: "no ambient signal — AAAK requires focus_wing or recent_activity".into(),
                confidence: 0.0,
                recognition_token_cost: 0,
            });
        }

        // Restore AaakOpts::default() — context filter replaces threshold workaround
        let budget = budget_remaining.min(self.max_tokens);
        let result = self.brain.aaak(AaakOpts {
            max_tokens: budget,
            include_wings: Some(relevant_wings.into_iter().collect()),
            ..AaakOpts::default()
        })?;

        if result.fact_count == 0 {
            return Ok(LayerResult::Skipped {
                reason: "no foundational facts in relevant wings".into(),
                confidence: 0.0,
                recognition_token_cost: 0,
            });
        }

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

        Ok(LayerResult::Sufficient {
            hits: vec![hit],
            tokens_used: result.estimated_tokens,
            confidence: 0.95,
            recognition_token_cost: 0,
        })
    }
}

// ── L2: EpisodeLayer ───────────────────────────────────────────────

/// L2 Episode layer: retrieves memories grouped by episode_id.
///
/// Runs FTS, groups results by episode_id, scores episodes by
/// sum of constituent signal scores × ambient boost, returns the
/// top episode's full memory set.
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
        context: &RecognitionContext,
    ) -> Result<LayerResult, Box<dyn std::error::Error + Send + Sync>> {
        let recall = self
            .brain
            .recall_local(query)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

        // Group FTS hits by episode_id, sum signal scores per episode
        let mut episode_scores: HashMap<String, f64> = HashMap::new();
        let mut episode_wings: HashMap<String, String> = HashMap::new();
        for hit in &recall.memory_hits {
            if let Some(ref ep_id) = hit.episode_id {
                *episode_scores.entry(ep_id.clone()).or_default() += hit.signal_score;
                // Track wing for the episode (first hit's wing wins)
                if let Some(ref w) = hit.wing {
                    episode_wings
                        .entry(ep_id.clone())
                        .or_insert_with(|| w.clone());
                }
            }
        }

        if episode_scores.is_empty() {
            return Ok(LayerResult::Skipped {
                reason: "no memories with episode_id matched".into(),
                confidence: 0.0,
                recognition_token_cost: 0,
            });
        }

        // Apply ambient boost to episode scores
        // Use episode metadata (wing, started_at) from the brain's episode list
        let episodes = self.brain.list_episodes(None, 1000).unwrap_or_default();
        let episode_meta: HashMap<&str, (&str, Option<&str>)> = episodes
            .iter()
            .map(|e| {
                (
                    e.id.as_str(),
                    (e.wing.as_str(), Some(e.started_at.as_str())),
                )
            })
            .collect();

        let mut boosted_scores: Vec<(String, f64)> = episode_scores
            .into_iter()
            .map(|(ep_id, query_score)| {
                let (wing, started_at) = episode_meta
                    .get(ep_id.as_str())
                    .copied()
                    .or_else(|| episode_wings.get(&ep_id).map(|w| (w.as_str(), None)))
                    .unwrap_or(("unknown", None));
                let boost = ambient_boost_for_episode(wing, started_at, context);
                (ep_id, query_score * boost)
            })
            .collect();

        // Sort by boosted score descending
        boosted_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (top_id, top_score) = boosted_scores[0].clone();
        let second_score = boosted_scores.get(1).map(|x| x.1).unwrap_or(0.0);

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
                recognition_token_cost: 0,
            })
        } else {
            Ok(LayerResult::Partial {
                hits,
                tokens_used,
                confidence: 0.5,
                recognition_token_cost: 0,
            })
        }
    }
}

// ── L3: ConstellationLayer ─────────────────────────────────────────

/// L3 Constellation/TACT layer: fingerprint + FTS recall.
///
/// Applies ambient boost based on memory wing alignment with context.
/// Constellation fingerprints have a single wing field (not per-peak),
/// so alignment is based on each hit's wing matching context wings.
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
        context: &RecognitionContext,
    ) -> Result<LayerResult, Box<dyn std::error::Error + Send + Sync>> {
        let result = self
            .brain
            .recall_local(query)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

        if result.memory_hits.is_empty() {
            return Ok(LayerResult::Skipped {
                reason: "no constellation/FTS matches".into(),
                confidence: 0.0,
                recognition_token_cost: 0,
            });
        }

        // Apply ambient boost and sort by boosted score
        let mut boosted_hits: Vec<(MemoryHit, f64)> = result
            .memory_hits
            .into_iter()
            .map(|hit| {
                let boost = ambient_boost_for_hit(&hit, context);
                let boosted_score = hit.signal_score * boost;
                (hit, boosted_score)
            })
            .collect();

        boosted_hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Estimate tokens: ~4 chars per token
        let budget = budget_remaining.min(self.max_tokens);
        let max_chars = budget * 4;

        let mut chars_used = 0;
        let mut hits = Vec::new();
        for (mut hit, boosted_score) in boosted_hits {
            let hit_chars = hit.content.len() + hit.key.len() + 20;
            if chars_used + hit_chars > max_chars && !hits.is_empty() {
                break;
            }
            chars_used += hit_chars;
            // Store boosted score as the hit's signal_score for downstream ranking
            hit.signal_score = boosted_score;
            hits.push(hit);
        }

        let tokens_used = chars_used / 4;

        // Confidence proportional to top hit's boosted score, capped
        let confidence = hits
            .first()
            .map(|h| h.signal_score.min(0.85))
            .unwrap_or(0.0);

        Ok(LayerResult::Partial {
            hits,
            tokens_used,
            confidence,
            recognition_token_cost: 0,
        })
    }
}
