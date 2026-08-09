//! Cascade integrated retrieval pipeline for spectral-graph.
//!
//! Single-pipeline design: FTS K=40 → ambient boost → signal/recency re-ranking
//! → dedup → episode diversity. All subsystems contribute to one result set.
//! (Dedup precedes diversity so dedup's score re-sort cannot clobber the
//! diversity interleave.)

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

    if let Some(created_utc) = hit
        .created_at
        .as_deref()
        .and_then(crate::ranking::parse_created_at)
    {
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
    /// Strengthen ambient disambiguation only when lexical evidence is tied.
    /// A query with a decisive top lexical match keeps the conservative
    /// weights so context cannot hijack an explicit request.
    pub adaptive_ambient: bool,
    /// Apply signal_score re-ranking. Default true.
    pub apply_signal_reranking: bool,
    /// Apply recency decay. Default true.
    pub apply_recency: bool,
    /// Recency half-life in days. Default 365.
    pub recency_half_life_days: f64,
    /// Apply episode diversity (cap per episode). Default **false**: the
    /// interleave is set-neutral (no session/key-recall change) and its
    /// actor-accuracy impact is unvalidated, so it stays off pending an
    /// end-to-end A/B. See the `Default` impl for the full rationale.
    pub apply_episode_diversity: bool,
    /// Max memories per episode in top results. Default 5.
    pub max_per_episode: usize,
    /// Apply context chain dedup. Default true.
    pub apply_context_dedup: bool,
    /// G4: additive proximity boost. Default false pending measurement.
    pub apply_proximity: bool,
    /// Weight for the proximity boost.
    pub proximity_weight: f64,
    /// Compose signals by reciprocal rank fusion instead of additive boosts.
    pub use_rrf: bool,
    /// Per-channel RRF weights, applied only when `use_rrf` is set. See
    /// [`crate::brain::RecallTopKConfig`] for why these are worth varying.
    pub rrf_bm25_weight: f64,
    pub rrf_declarative_weight: f64,
    pub rrf_proximity_weight: f64,
    pub rrf_recency_weight: f64,
    pub rrf_signal_weight: f64,
    pub rrf_entity_weight: f64,
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
    /// Perform the recall→feedback write-back (auto-reinforce every returned
    /// memory + log a `RetrievalEvent`). Default **true** — the historical
    /// behavior every `recall_*` entry point relies on.
    ///
    /// Set false for a strictly read-only retrieval whose reinforcement is
    /// deferred until the caller reports what it actually used. Auto-reinforce
    /// credits *exposure*, not usefulness: it strengthens all `k` hits before
    /// the consumer has filtered them, which is the mechanism behind the
    /// measured popularity bias (728/744 events returning ~the same 40
    /// memories; see docs/internal/LAST_LOOK.md). `Brain::turn` clears this
    /// flag and reinforces only memories reported `Used`.
    pub write_back: bool,
}

impl Default for CascadePipelineConfig {
    fn default() -> Self {
        Self {
            k: 40,
            apply_ambient_boost: true,
            ambient_weights: AmbientBoostWeights::default(),
            adaptive_ambient: true,
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
            apply_proximity: false,
            proximity_weight: crate::ranking::PROXIMITY_WEIGHT_DEFAULT,
            use_rrf: false,
            rrf_bm25_weight: 1.0,
            rrf_declarative_weight: 1.0,
            rrf_proximity_weight: 1.0,
            rrf_recency_weight: 1.0,
            rrf_signal_weight: 1.0,
            rrf_entity_weight: 1.0,
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
            // Preserves the historical recall behavior. Only the deferred-outcome
            // turn path clears it.
            write_back: true,
        }
    }
}

// Colocated with the config it exercises; pipeline items follow intentionally.
#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod config_tests {
    use super::*;

    fn hit(key: &str, content: &str) -> MemoryHit {
        MemoryHit {
            id: key.into(),
            key: key.into(),
            content: content.into(),
            wing: None,
            hall: None,
            signal_score: 0.5,
            visibility: "private".into(),
            hits: 1,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            declarative_density: None,
            description: None,
            source_brain_id: None,
            signature: None,
        }
    }

    #[test]
    fn default_fetch_mult_is_off_pending_actor_validation() {
        // The widening capability exists but defaults OFF (1): retrieval-Pareto-
        // safe, but the end-to-end actor A/B was inconclusive/directionally
        // negative (see the field comment + doc). Locked at 1 so it is not
        // silently re-defaulted to 3 without a deterministic, powered actor
        // validation. Flip to 3 only alongside that evidence.
        assert_eq!(CascadePipelineConfig::default().fetch_mult, 1);
    }

    #[test]
    fn adaptive_ambient_detects_lexically_decisive_query() {
        let hits = vec![
            hit("work:notes", "sprint deploy checklist and open bugs"),
            hit("cooking:notes", "recipe garlic and salt"),
        ];
        assert!(ambient_query_is_decisive(
            "sprint deploy checklist bugs",
            &hits
        ));
    }

    #[test]
    fn adaptive_ambient_treats_shared_terms_as_ambiguous() {
        let hits = vec![
            hit("work:notes", "the sprint review notes"),
            hit("cooking:notes", "the recipe review notes"),
        ];
        assert!(!ambient_query_is_decisive("review notes", &hits));
    }
}

/// Run the integrated cascade pipeline (no visibility boundary — returns every
/// hit, equivalent to a `Private` context). Thin wrapper over
/// [`run_cascade_pipeline_scoped`]; kept for callers that query their own brain.
pub fn run_cascade_pipeline(
    brain: &Brain,
    query: &str,
    context: &RecognitionContext,
    config: &CascadePipelineConfig,
) -> Result<Vec<MemoryHit>, crate::Error> {
    run_cascade_pipeline_scoped(
        brain,
        query,
        context,
        config,
        spectral_core::visibility::Visibility::Private,
    )
}

/// Run the integrated cascade pipeline with a visibility boundary.
///
/// TACT retrieval at K=40 → ambient boost → signal/recency re-ranking
/// → context dedup → episode diversity. TACT provides the full tiered search
/// (fingerprint → wing → FTS) as the entry point. (Dedup precedes diversity so
/// dedup's score re-sort does not clobber the diversity interleave.)
///
/// `visibility` filters candidates to those whose own label admits this context
/// (`content >= context`), applied before reranking/truncation.
pub fn run_cascade_pipeline_scoped(
    brain: &Brain,
    query: &str,
    context: &RecognitionContext,
    config: &CascadePipelineConfig,
    visibility: spectral_core::visibility::Visibility,
) -> Result<Vec<MemoryHit>, crate::Error> {
    // Step 1: TACT + FTS combined retrieval — TACT for classified queries,
    // FTS supplement when TACT returns fewer than K results. Widen the
    // candidate pool to k×fetch_mult so reranking can promote answer keys
    // buried below FTS rank k; the output is truncated back to k below, so
    // context size tracks k, not the pool.
    let fetch_k = config.k.saturating_mul(config.fetch_mult.max(1));
    // Ambient scope reaches TACT's tier selection here. Without it a wing is
    // detected from query text alone, which covers 12.4% of real agent queries
    // — only the ones that name a project. `None` reproduces prior behaviour
    // exactly, so an empty context changes nothing.
    let mut candidates = brain
        .cascade_retrieve_scoped(query, fetch_k, context.focus_wing.as_deref())
        .map_err(|e| crate::Error::Schema(e.to_string()))?;

    // Visibility boundary: keep only hits whose own label admits this context
    // (`content >= context`). Applied here — before reranking/truncation, as
    // recall_topk_fts does — so the visible top-k is filled from the full pool
    // rather than diluted by filtered-out private hits. A Private context
    // admits everything, so the unscoped entry point is unaffected.
    candidates.retain(|h| crate::brain::str_to_vis(&h.visibility).allows(visibility));

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // Step 2: Unified re-ranking pipeline (same implementation as topk_fts)
    let ambient_weights = if config.apply_ambient_boost
        && config.adaptive_ambient
        && !ambient_query_is_decisive(query, &candidates)
    {
        AmbientBoostWeights {
            // 0.45 clears both ambiguity fixtures; the lexical decisiveness
            // gate prevents this stronger damp from touching explicit queries.
            mismatch_penalty: 0.45,
            ..config.ambient_weights
        }
    } else {
        config.ambient_weights
    };
    let reranking_config = crate::ranking::RerankingConfig {
        apply_signal_score: config.apply_signal_reranking,
        signal_score_weight: 0.3,
        apply_recency: config.apply_recency,
        recency_half_life_days: config.recency_half_life_days,
        apply_entity_boost: false,
        entity_boost_weight: 0.05,
        apply_ambient_boost: config.apply_ambient_boost,
        ambient_weights,
        apply_declarative_boost: config.apply_declarative_boost,
        declarative_weight: 0.10,
        co_retrieval_weight: config.co_retrieval_weight,
        apply_episode_diversity: config.apply_episode_diversity,
        max_per_episode: config.max_per_episode,
        apply_context_dedup: config.apply_context_dedup,
        // G4: the cascade is the path our only consumer actually calls
        // (Permagent's `recall_cascade`), so proximity is wired here too
        // rather than being a topk_fts-only lever.
        apply_proximity: config.apply_proximity,
        proximity_weight: config.proximity_weight,
        use_rrf: config.use_rrf,
        rrf_bm25_weight: config.rrf_bm25_weight,
        rrf_declarative_weight: config.rrf_declarative_weight,
        rrf_proximity_weight: config.rrf_proximity_weight,
        rrf_recency_weight: config.rrf_recency_weight,
        rrf_signal_weight: config.rrf_signal_weight,
        rrf_entity_weight: config.rrf_entity_weight,
        ..Default::default()
    };

    // Only compute co-retrieval affinity when it will actually be applied.
    // Each computation issues one related_memories DB query per anchor (×3);
    // with co_retrieval_weight at its default 0.0 the boosts multiply to zero
    // and are discarded, so skip the queries entirely on the hot path.
    let co_boosts = if reranking_config.co_retrieval_weight > 0.0 {
        crate::ranking::compute_co_retrieval_boosts(brain, &candidates, 3)
    } else {
        std::collections::HashMap::new()
    };

    let query_words = crate::brain::fts_query_words_opts(query, true);
    let mut results = crate::ranking::apply_reranking_pipeline_with_query(
        candidates,
        &reranking_config,
        context,
        &co_boosts,
        &query_words,
    );
    // Truncate the widened pool back to k after reranking: the pool exists only
    // so reranking can surface buried keys; callers and context budget see k.
    results.truncate(config.k);

    // ── Recall→Recognition feedback: auto-reinforce + event logging ──
    // Both are best-effort: failures never block retrieval. Both are
    // skipped entirely on a read-only brain: federated read-time fan-out
    // must not mutate a member's ranking state (score inflation, decay
    // clock resets) or write the caller's query metadata into its store.
    // Also skipped when the caller opted out (`write_back: false`) because it
    // will report outcomes and reinforce only what was actually used.
    if config.write_back && !brain.is_read_only() {
        // Auto-reinforce returned memories (small strength nudge) + log the
        // retrieval event for co-access mining. Repeated retrievals accumulate;
        // this makes the Archivist's decay/boost loop functional without
        // caller-explicit reinforcement. Batched into one transaction and, when
        // async write-back is enabled, spawned off the recall critical path.
        const AUTO_REINFORCE_STRENGTH: f64 = 0.01;
        let keys: Vec<String> = results.iter().map(|h| h.key.clone()).collect();
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
        brain.write_back(keys, event, AUTO_REINFORCE_STRENGTH);
    }

    // Associative recall spreading (opt-in via config.spread; OFF by default =
    // no-op). Applied last so reinforce/logging act on the core recall set and
    // the associatively-recovered mates only augment the returned context.
    crate::spreading::associative_spread(brain, &mut results, &config.spread, visibility);

    Ok(results)
}

/// Whether the first lexical candidate has a clear query-term advantage over
/// every alternative. Ambient context is a disambiguator, so it should become
/// stronger only when lexical evidence is tied; a decisive explicit query
/// keeps the conservative penalty that is proven not to hijack requests.
fn ambient_query_is_decisive(query: &str, candidates: &[MemoryHit]) -> bool {
    if candidates.len() < 2 {
        return true;
    }
    let terms: HashSet<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|t| t.len() > 2 && !matches!(t.as_str(), "the" | "and" | "for" | "with"))
        .collect();
    if terms.len() < 2 {
        return false;
    }
    let overlap = |hit: &MemoryHit| {
        let text = format!("{} {}", hit.key, hit.content).to_lowercase();
        let tokens: HashSet<&str> = text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .collect();
        terms
            .iter()
            .filter(|term| tokens.contains(term.as_str()))
            .count()
    };
    let first = overlap(&candidates[0]);
    let runner_up = candidates[1..].iter().map(overlap).max().unwrap_or(0);
    first >= 2 && first > runner_up
}
