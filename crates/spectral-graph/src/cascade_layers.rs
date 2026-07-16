//! Cascade integrated retrieval pipeline for spectral-graph.
//!
//! Single-pipeline design: FTS K=40 → ambient boost → signal/recency re-ranking
//! → episode diversity → dedup. All subsystems contribute to one result set.

use std::collections::{HashMap, HashSet};

use spectral_cascade::RecognitionContext;
use spectral_ingest::MemoryHit;

use crate::brain::Brain;

// ── Ambient boost ───────────────────────────────────────────────────

/// Tunable weights for the ambient context boost. Defaults preserve the
/// original hardcoded behavior (wing-match ×1.5, mismatch ×0.7, fresh-hour
/// ×1.3, fresh-day ×1.1, clamp [0.5, 2.0]). Exposed so the disambiguation
/// strength can be tuned per deployment: `ambient_weight_sweep` (bench-real)
/// maps the tradeoff frontier between context disambiguation and wrongly
/// overriding a strong relevance signal.
#[derive(Debug, Clone, Copy)]
pub struct AmbientBoostWeights {
    /// Multiplier when the hit's wing matches focus/recent activity.
    pub wing_match: f64,
    /// Multiplier for non-matching hits under strong context (focus or activity present).
    pub mismatch_penalty: f64,
    /// Multiplier for hits created within the last hour.
    pub fresh_hour: f64,
    /// Multiplier for hits created within the last day (but not hour).
    pub fresh_day: f64,
    /// Final clamp bounds.
    pub clamp_min: f64,
    pub clamp_max: f64,
}

impl Default for AmbientBoostWeights {
    /// **Penalty-only ambient** — the measured frontier point. The
    /// `ambient_weight_sweep` bench maps disambiguation (A, 12 ambiguous
    /// query/focus cases) against explicit-override respect (B, 6 cases where
    /// the query unambiguously names an out-of-context memory):
    ///
    /// | weights | A | B |
    /// |---|---|---|
    /// | 1.0 / 0.7 (this default) | 11/12 | **6/6** |
    /// | 1.5 / 0.7 (previous)     | 12/12 | 5/6 (hijacks an explicit query) |
    ///
    /// Boosting in-context hits (`wing_match > 1`) buys the last ambiguity case
    /// by inflating context above content relevance — which then also overrides
    /// queries that explicitly ask for something outside the current context,
    /// the trust-breaking failure. Damping out-of-context hits instead
    /// (`mismatch_penalty < 1`) disambiguates nearly as well and never hijacks:
    /// an explicit query's strong relevance survives a 0.7 damp, ambient noise
    /// does not. Consumers with real usage data can re-tune via
    /// [`CascadePipelineConfig::ambient_weights`].
    fn default() -> Self {
        Self {
            wing_match: 1.0,
            mismatch_penalty: 0.7,
            fresh_hour: 1.3,
            fresh_day: 1.1,
            clamp_min: 0.5,
            clamp_max: 2.0,
        }
    }
}

/// Compute ambient boost for a memory hit based on wing alignment and recency,
/// with default weights. Returns a value in `[clamp_min, clamp_max]`. Identity
/// (1.0) when context is empty.
pub fn ambient_boost_for_hit(hit: &MemoryHit, context: &RecognitionContext) -> f64 {
    ambient_boost_with(hit, context, &AmbientBoostWeights::default())
}

/// [`ambient_boost_for_hit`] with explicit weights.
pub fn ambient_boost_with(
    hit: &MemoryHit,
    context: &RecognitionContext,
    w: &AmbientBoostWeights,
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

    let hit_wing = hit.wing.as_deref();

    let wing_match = hit_wing.is_some()
        && (context.focus_wing.as_deref() == hit_wing
            || hit_wing.is_some_and(|w| activity_wings.contains(w)));

    if wing_match {
        boost *= w.wing_match;
    }

    if let Some(created_utc) = hit.created_at.as_deref().and_then(crate::ranking::parse_created_at) {
        let age_minutes = (context.now - created_utc).num_minutes();
        if (0..60).contains(&age_minutes) {
            boost *= w.fresh_hour;
        } else if (60..1440).contains(&age_minutes) {
            boost *= w.fresh_day;
        }
    }

    let has_strong_context = context.focus_wing.is_some() || !context.recent_activity.is_empty();
    if has_strong_context && !wing_match {
        boost *= w.mismatch_penalty;
    }

    boost.clamp(w.clamp_min, w.clamp_max)
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
    /// Weights for the ambient boost.
    pub ambient_weights: AmbientBoostWeights,
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
    /// Additive weight for the co-retrieval (cross-query co-access) boost.
    /// Default **0.0** (disabled): measured on Permagent's real workload, a
    /// non-zero weight degrades top-5 relevance (~3–4.5:1 worse, p≈0) because a
    /// dense generic co-access blob induces popularity bias. Kept as an opt-in
    /// knob for retuning. See docs/internal/tickets/coretrieval-regression.md.
    pub co_retrieval_weight: f64,
    /// Candidate-pool widening: fetch `k × fetch_mult` FTS/TACT candidates,
    /// rerank the wider pool, then truncate to `k`. Default **1** (no widening,
    /// preserves prior behavior). The narrow `k`-only fetch means answer keys
    /// beyond FTS rank `k` are structurally unreachable regardless of
    /// reranking; widening lets signal/recency/declarative reranking PROMOTE
    /// buried keys into the top-k at constant output size (tokens track `k`, not
    /// the pool). Mirrors `RecallTopKConfig::fetch_mult` on the topk_fts path.
    /// Highest leverage on Counting/multi-session answer-KEY completeness.
    pub fetch_mult: usize,
    /// Apply the declarative-density boost (rewards declarative/factual phrasing).
    /// Default true. Counterproductive for Counting: it demotes the terse event
    /// instances a count must enumerate. Exposed so profiles can disable it.
    pub apply_declarative_boost: bool,
    /// Associative recall spreading applied to the final results. Default OFF
    /// (`SpreadMode::Off`, a no-op). When enabled, follows co-occurrence links
    /// (episode / cross-session) to recover memories that share no words with the
    /// query — the vocabulary gap FTS cannot cross. See [`crate::spreading`].
    pub spread: crate::spreading::AssocSpreadConfig,
}

impl Default for CascadePipelineConfig {
    fn default() -> Self {
        Self {
            k: 40,
            apply_ambient_boost: true,
            ambient_weights: AmbientBoostWeights::default(),
            apply_signal_reranking: true,
            apply_recency: true,
            recency_half_life_days: 365.0,
            // DEFAULT off. This flag was `true` but de-facto inert: context dedup
            // (below) re-sorts by score and, running last, exactly restored the
            // pre-diversity order — so diversity never affected output. The dedup
            // ordering bug is fixed (ranking.rs runs dedup before diversity), which
            // means enabling this now *would* reorder the top-k on multi-session
            // shapes. That reorder is set-neutral (no session/key-recall change) so
            // the oracle cannot gate it; leaving it off preserves the validated
            // runtime behavior. Enable only behind an end-to-end actor A/B.
            apply_episode_diversity: false,
            max_per_episode: 5,
            apply_context_dedup: true,
            co_retrieval_weight: 0.0,
            // CAPABILITY present, DEFAULT off (1). Widening to 3×k is measured
            // Pareto-safe on RETRIEVAL (token-neutral; recovers buried answers
            // where session-recall has headroom, e.g. single-session-preference
            // 93.3%→96.7%), BUT the end-to-end actor A/B did not validate it: on
            // single-session-preference (n=30, sonnet-4-6) fm=3 scored 14 fails
            // vs fm=1's 11 — directionally worse, though the run was inconclusive
            // (actor temperature was unpinned → sampling noise swamps a ~5/30
            // retrieval delta). Per project discipline we do not ship a default
            // behavior change on a retrieval proxy alone. Opt in via config or
            // SPECTRAL_CASCADE_FETCH_MULT; re-default only after a deterministic
            // (temp=0), adequately-powered actor validation shows a gain.
            // See docs/internal/cascade-fetch-mult-lever-2026-07-14.md.
            fetch_mult: 1,
            apply_declarative_boost: true,
            spread: crate::spreading::AssocSpreadConfig::default(),
        }
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn default_fetch_mult_is_off_pending_actor_validation() {
        // The widening capability exists but defaults OFF (1): retrieval-Pareto-
        // safe, but the end-to-end actor A/B was inconclusive/directionally
        // negative (see the field comment + doc). Locked at 1 so it is not
        // silently re-defaulted to 3 without a deterministic, powered actor
        // validation. Flip to 3 only alongside that evidence.
        assert_eq!(CascadePipelineConfig::default().fetch_mult, 1);
    }
}

/// Run the integrated cascade pipeline.
///
/// TACT retrieval at K=40 → ambient boost → signal/recency re-ranking
/// → context dedup → episode diversity. TACT provides the full tiered search
/// (fingerprint → wing → FTS) as the entry point. (Dedup precedes diversity so
/// dedup's score re-sort does not clobber the diversity interleave.)
pub fn run_cascade_pipeline(
    brain: &Brain,
    query: &str,
    context: &RecognitionContext,
    config: &CascadePipelineConfig,
) -> Result<Vec<MemoryHit>, crate::Error> {
    // Step 1: TACT + FTS combined retrieval — TACT for classified queries,
    // FTS supplement when TACT returns fewer than K results. Widen the
    // candidate pool to k×fetch_mult so reranking can promote answer keys
    // buried below FTS rank k; the output is truncated back to k below, so
    // context size tracks k, not the pool.
    let fetch_k = config.k.saturating_mul(config.fetch_mult.max(1));
    let candidates = brain
        .cascade_retrieve(query, fetch_k)
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
        ambient_weights: config.ambient_weights,
        apply_declarative_boost: config.apply_declarative_boost,
        declarative_weight: 0.10,
        co_retrieval_weight: config.co_retrieval_weight,
        apply_episode_diversity: config.apply_episode_diversity,
        max_per_episode: config.max_per_episode,
        apply_context_dedup: config.apply_context_dedup,
    };

    let co_boosts = crate::ranking::compute_co_retrieval_boosts(brain, &candidates, 3);

    let mut results = crate::ranking::apply_reranking_pipeline(
        candidates,
        &reranking_config,
        context,
        &co_boosts,
    );
    // Truncate the widened pool back to k after reranking: the pool exists only
    // so reranking can surface buried keys; callers and context budget see k.
    results.truncate(config.k);

    // ── Recall→Recognition feedback: auto-reinforce + event logging ──
    // Both are best-effort: failures never block retrieval. Both are
    // skipped entirely on a read-only brain: federated read-time fan-out
    // must not mutate a member's ranking state (score inflation, decay
    // clock resets) or write the caller's query metadata into its store.
    if !brain.is_read_only() {
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
            query_hash: spectral_ingest::hash_query(query),
            timestamp: chrono::Utc::now().to_rfc3339(),
            memory_ids_json: serde_json::to_string(&memory_ids).unwrap_or_default(),
            method: "cascade".into(),
            wing: results.first().and_then(|h| h.wing.clone()),
            question_type: None, // Set by bench caller if applicable
            session_id: context.session_id.clone(),
        };
        let _ = brain.log_retrieval_event(&event);
    }

    // Associative recall spreading (opt-in via config.spread; OFF by default =
    // no-op). Applied last so reinforce/logging act on the core recall set and
    // the associatively-recovered mates only augment the returned context.
    crate::spreading::associative_spread(brain, &mut results, &config.spread);

    Ok(results)
}
