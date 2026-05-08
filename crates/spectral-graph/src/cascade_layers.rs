//! Cascade integrated retrieval pipeline for spectral-graph.
//!
//! Single-pipeline design: FTS K=40 → ambient boost → signal/recency re-ranking
//! → episode diversity → dedup. All subsystems contribute to one result set.

use std::collections::{HashMap, HashSet};

use spectral_cascade::RecognitionContext;
use spectral_ingest::MemoryHit;

use crate::brain::Brain;

// ── Ambient boost ───────────────────────────────────────────────────

/// Compute ambient boost for a memory hit based on wing alignment and recency.
/// Returns a value in [0.5, 2.0]. Identity (1.0) when context is empty.
pub fn ambient_boost_for_hit(hit: &MemoryHit, context: &RecognitionContext) -> f64 {
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

    let wing_match = hit_wing.is_some()
        && (context.focus_wing.as_deref() == hit_wing
            || hit_wing.is_some_and(|w| activity_wings.contains(w)));

    if wing_match {
        boost *= 1.5;
    }

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

    let has_strong_context = context.focus_wing.is_some() || !context.recent_activity.is_empty();
    if has_strong_context && !wing_match {
        boost *= 0.7;
    }

    boost.clamp(0.5, 2.0)
}

// ── Episode diversity re-ranking ────────────────────────────────────

/// Re-rank hits to ensure episode/session diversity in the top results.
/// Instead of collapsing to one episode, interleave memories from different
/// episodes so the top-K spans multiple sessions.
pub fn apply_episode_diversity(hits: &mut Vec<MemoryHit>, max_per_episode: usize) {
    if hits.is_empty() || max_per_episode == 0 {
        return;
    }

    // Count per episode, cap each
    let mut episode_counts: HashMap<String, usize> = HashMap::new();
    let mut result = Vec::with_capacity(hits.len());
    let mut overflow = Vec::new();

    for hit in hits.drain(..) {
        let ep_key = hit
            .episode_id
            .clone()
            .or_else(|| {
                // Fallback: use session prefix from key as episode proxy
                hit.key.split(':').next().map(|s| s.to_string())
            })
            .unwrap_or_default();

        let count = episode_counts.entry(ep_key).or_default();
        if *count < max_per_episode {
            *count += 1;
            result.push(hit);
        } else {
            overflow.push(hit);
        }
    }

    // Append overflow at the end (available but deprioritized)
    result.extend(overflow);
    *hits = result;
}

// ── Integrated pipeline ─────────────────────────────────────────────

/// Configuration for the integrated cascade pipeline.
#[derive(Debug, Clone)]
pub struct CascadePipelineConfig {
    /// Number of FTS candidates to retrieve. Default 40.
    pub k: usize,
    /// Apply ambient boost from RecognitionContext. Default true.
    pub apply_ambient_boost: bool,
    /// Apply signal_score re-ranking. Default true.
    pub apply_signal_reranking: bool,
    /// Apply recency decay. Default true.
    pub apply_recency: bool,
    /// Recency half-life in days. Default 365.
    pub recency_half_life_days: f64,
    /// Apply episode diversity (cap per episode). Default true.
    pub apply_episode_diversity: bool,
    /// Max memories per episode in top results. Default 5.
    pub max_per_episode: usize,
    /// Apply context chain dedup. Default true.
    pub apply_context_dedup: bool,
}

impl Default for CascadePipelineConfig {
    fn default() -> Self {
        Self {
            k: 40,
            apply_ambient_boost: true,
            apply_signal_reranking: true,
            apply_recency: true,
            recency_half_life_days: 365.0,
            apply_episode_diversity: true,
            max_per_episode: 5,
            apply_context_dedup: true,
        }
    }
}

/// Run the integrated cascade pipeline.
///
/// TACT retrieval at K=40 → ambient boost → signal/recency re-ranking
/// → episode diversity → context dedup. TACT provides the full tiered search
/// (fingerprint → wing → FTS) as the entry point.
pub fn run_cascade_pipeline(
    brain: &Brain,
    query: &str,
    context: &RecognitionContext,
    config: &CascadePipelineConfig,
) -> Result<Vec<MemoryHit>, crate::Error> {
    // Step 1: TACT + FTS combined retrieval — TACT for classified queries,
    // FTS supplement when TACT returns fewer than K results
    let candidates = brain
        .cascade_retrieve(query, config.k)
        .map_err(|e| crate::Error::Schema(e.to_string()))?;

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // Step 2: Unified re-ranking pipeline (same implementation as topk_fts)
    let reranking_config = crate::ranking::RerankingConfig {
        apply_signal_score: config.apply_signal_reranking,
        signal_score_weight: 0.3,
        apply_recency: config.apply_recency,
        recency_half_life_days: config.recency_half_life_days,
        apply_entity_boost: false,
        entity_boost_weight: 0.05,
        apply_ambient_boost: config.apply_ambient_boost,
        apply_declarative_boost: true,
        declarative_weight: 0.10,
        apply_episode_diversity: config.apply_episode_diversity,
        max_per_episode: config.max_per_episode,
        apply_context_dedup: config.apply_context_dedup,
    };

    let results = crate::ranking::apply_reranking_pipeline(candidates, &reranking_config, context);

    // ── Recall→Recognition feedback: auto-reinforce + event logging ──
    // Both are best-effort: failures never block retrieval.

    // Auto-reinforce returned memories with a small strength nudge.
    // Repeated retrievals accumulate; this makes the Archivist's
    // decay/boost loop functional without caller-explicit reinforcement.
    const AUTO_REINFORCE_STRENGTH: f64 = 0.01;
    for hit in &results {
        let _ = brain.reinforce_by_id(&hit.key, AUTO_REINFORCE_STRENGTH);
    }

    // Log retrieval event for future co-access mining / pattern detection.
    let memory_ids: Vec<&str> = results.iter().map(|h| h.id.as_str()).collect();
    let event = spectral_ingest::RetrievalEvent {
        query_hash: format!(
            "{:016x}",
            blake3::hash(query.as_bytes()).as_bytes()[..8]
                .iter()
                .fold(0u64, |acc, &b| (acc << 8) | b as u64)
        ),
        timestamp: chrono::Utc::now().to_rfc3339(),
        memory_ids_json: serde_json::to_string(&memory_ids).unwrap_or_default(),
        method: "cascade".into(),
        wing: results.first().and_then(|h| h.wing.clone()),
        question_type: None, // Set by bench caller if applicable
    };
    let _ = brain.log_retrieval_event(&event);

    Ok(results)
}
