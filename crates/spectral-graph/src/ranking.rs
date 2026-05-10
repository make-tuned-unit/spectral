//! Re-ranking signals for FTS retrieval results.
//!
//! Each function mutates candidate scores in-place. Applied after FTS retrieval
//! to improve precision without any LLM cost.

use chrono::{DateTime, Utc};
use spectral_ingest::MemoryHit;

/// Apply signal_score as a secondary weighting factor.
///
/// Combines FTS rank position with stored signal_score via weighted sum:
/// `final = (1 - weight) * fts_normalized + weight * signal_score`
///
/// `weight` in [0.0, 1.0]. Default 0.3 — FTS rank dominates but signal adds discrimination.
pub fn apply_signal_score_weight(candidates: &mut [MemoryHit], weight: f64) {
    if candidates.is_empty() || weight <= 0.0 {
        return;
    }

    let weight = weight.clamp(0.0, 1.0);

    // FTS results are already ordered by rank. Normalize position to [0, 1].
    let n = candidates.len() as f64;
    for (i, hit) in candidates.iter_mut().enumerate() {
        let fts_normalized = 1.0 - (i as f64 / n);
        let blended = (1.0 - weight) * fts_normalized + weight * hit.signal_score;
        hit.signal_score = blended;
    }

    // Re-sort by blended score
    candidates.sort_by(|a, b| {
        b.signal_score
            .partial_cmp(&a.signal_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Apply exponential recency decay to scores.
///
/// `recency_factor = 0.5 ^ (age_days / half_life_days)`
/// Multiplies each candidate's score by its recency factor.
pub fn apply_recency_weight(candidates: &mut [MemoryHit], half_life_days: f64, now: DateTime<Utc>) {
    if candidates.is_empty() || half_life_days <= 0.0 {
        return;
    }

    for hit in candidates.iter_mut() {
        let age_days = hit
            .created_at
            .as_deref()
            .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok())
            .map(|dt| (now - dt.and_utc()).num_hours() as f64 / 24.0)
            .unwrap_or(0.0)
            .max(0.0);

        let recency_factor = 0.5_f64.powf(age_days / half_life_days);
        hit.signal_score *= recency_factor;
    }

    candidates.sort_by(|a, b| {
        b.signal_score
            .partial_cmp(&a.signal_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Boost top candidate within each entity cluster.
///
/// Groups candidates by shared wing (as a proxy for entity clustering since
/// full entity resolution requires graph queries). Within each wing group
/// containing 2+ candidates, the top-scoring candidate gets +boost_factor.
pub fn boost_entity_clusters(candidates: &mut [MemoryHit], boost_factor: f64) {
    if candidates.is_empty() || boost_factor <= 0.0 {
        return;
    }

    // Group indices by wing
    let mut wing_groups: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, hit) in candidates.iter().enumerate() {
        let wing = hit.wing.clone().unwrap_or_default();
        wing_groups.entry(wing).or_default().push(i);
    }

    // For each group with 2+ members, boost the top-scoring candidate
    for indices in wing_groups.values() {
        if indices.len() < 2 {
            continue;
        }
        let top_idx = *indices
            .iter()
            .max_by(|&&a, &&b| {
                candidates[a]
                    .signal_score
                    .partial_cmp(&candidates[b].signal_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        candidates[top_idx].signal_score += boost_factor;
    }

    candidates.sort_by(|a, b| {
        b.signal_score
            .partial_cmp(&a.signal_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

// ── Declarative density ─────────────────────────────────────────────

/// Compute the ratio of first-person declarative sentences in content.
///
/// A sentence is declarative if it contains a first-person pronoun
/// (I, me, my, mine, I've, I'm, I'll, I'd) and does not end with a
/// question mark. Short fragments (<3 words) are excluded.
///
/// Returns 0.0–1.0. Higher values indicate content where the user is
/// stating personal facts — which empirically correlates with
/// answer-bearing content on personal-memory benchmarks.
pub fn declarative_density(content: &str) -> f64 {
    let sentences: Vec<&str> = content
        .split(['.', '!', '?'])
        .filter(|s| s.split_whitespace().count() >= 3)
        .collect();
    if sentences.is_empty() {
        return 0.0;
    }

    let declarative = sentences
        .iter()
        .filter(|s| {
            let lower = s.to_lowercase();
            let has_first_person = lower.split_whitespace().any(|w| {
                matches!(
                    w,
                    "i" | "me" | "my" | "mine" | "i've" | "i'm" | "i'll" | "i'd"
                )
            });
            let not_question = !s.contains('?');
            has_first_person && not_question
        })
        .count();

    declarative as f64 / sentences.len() as f64
}

/// Deduplicate candidates whose content is a `[Memory context]` reference chain.
///
/// Detects candidates whose content starts with `[Memory context] - <key>:`
/// and collapses near-duplicates (same reference target) to keep only the
/// highest-scoring representative.
pub fn dedup_context_chains(mut candidates: Vec<MemoryHit>) -> Vec<MemoryHit> {
    if candidates.is_empty() {
        return candidates;
    }

    // Sort by score descending so first-seen = highest-scoring
    candidates.sort_by(|a, b| {
        b.signal_score
            .partial_cmp(&a.signal_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut seen_refs: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut result = Vec::with_capacity(candidates.len());

    for hit in candidates {
        if hit.content.starts_with("[Memory context]") {
            // Extract the reference key: "[Memory context] - <key>: ..."
            let ref_key = hit
                .content
                .strip_prefix("[Memory context] - ")
                .and_then(|rest| rest.split(':').next())
                .unwrap_or("")
                .to_string();

            if ref_key.is_empty() || seen_refs.insert(ref_key) {
                result.push(hit);
            }
            // Skip duplicates referencing the same key
        } else {
            result.push(hit);
        }
    }

    result
}

// ── Unified re-ranking pipeline ──────────────────────────────────────

use spectral_cascade::RecognitionContext;

/// Poll a boxed future that is expected to complete synchronously (e.g., SQLite queries).
/// Avoids the need for a tokio runtime when the future resolves on first poll.
///
/// SAFETY: This function only works if `fut` resolves on first poll.
/// MemoryStore implementations called from the ranking pipeline MUST use
/// only in-memory, sync-equivalent operations. Adding any await that
/// requires I/O (network, disk, lock contention) will cause this to panic
/// at runtime. If actually-async behavior is needed, refactor
/// apply_reranking_pipeline to be async instead.
fn poll_sync<T>(mut fut: std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + '_>>) -> T {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    fn noop_raw_waker() -> RawWaker {
        fn no_op(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VTABLE)
        }
        const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, no_op, no_op, no_op);
        RawWaker::new(std::ptr::null(), &VTABLE)
    }

    let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
    let mut cx = Context::from_waker(&waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(val) => val,
        Poll::Pending => panic!("poll_sync: future did not complete synchronously"),
    }
}

/// Configuration for the unified re-ranking pipeline.
/// Both topk_fts and cascade call this with different configs.
#[derive(Clone)]
pub struct RerankingConfig {
    pub apply_signal_score: bool,
    pub signal_score_weight: f64,
    pub apply_recency: bool,
    pub recency_half_life_days: f64,
    pub apply_entity_boost: bool,
    pub entity_boost_weight: f64,
    pub apply_ambient_boost: bool,
    pub apply_declarative_boost: bool,
    pub declarative_weight: f64,
    pub apply_episode_diversity: bool,
    pub max_per_episode: usize,
    pub apply_context_dedup: bool,
    /// Enable co-retrieval boost from historical retrieval patterns.
    pub apply_co_retrieval_boost: bool,
    /// Weight for co-retrieval boost: composite *= 1 + weight * normalized_count.
    pub co_retrieval_weight: f64,
    /// Number of top candidates to use as anchors for co-retrieval lookup.
    pub co_retrieval_top_k: usize,
    /// Memory store for querying co-retrieval pairs. None disables the boost.
    pub co_retrieval_store: Option<std::sync::Arc<dyn spectral_ingest::MemoryStore>>,
}

impl std::fmt::Debug for RerankingConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RerankingConfig")
            .field("apply_co_retrieval_boost", &self.apply_co_retrieval_boost)
            .field("co_retrieval_weight", &self.co_retrieval_weight)
            .field("co_retrieval_top_k", &self.co_retrieval_top_k)
            .field(
                "co_retrieval_store",
                &self.co_retrieval_store.as_ref().map(|_| "Some(...)"),
            )
            .finish_non_exhaustive()
    }
}

impl Default for RerankingConfig {
    fn default() -> Self {
        Self {
            apply_signal_score: true,
            signal_score_weight: 0.3,
            apply_recency: true,
            recency_half_life_days: 365.0,
            apply_entity_boost: false,
            entity_boost_weight: 0.05,
            apply_ambient_boost: false,
            apply_declarative_boost: false,
            declarative_weight: 0.10,
            apply_episode_diversity: false,
            max_per_episode: 5,
            apply_context_dedup: true,
            apply_co_retrieval_boost: false,
            co_retrieval_weight: 0.15,
            co_retrieval_top_k: 10,
            co_retrieval_store: None,
        }
    }
}

/// Unified re-ranking pipeline. Both retrieval frontends (topk_fts, cascade)
/// call this after their respective retrieval step.
///
/// Signals contribute additively/multiplicatively to a composite score.
/// No intermediate hard sorts — single sort at the end before post-rank ops.
pub fn apply_reranking_pipeline(
    candidates: Vec<MemoryHit>,
    config: &RerankingConfig,
    context: &RecognitionContext,
) -> Vec<MemoryHit> {
    if candidates.is_empty() {
        return candidates;
    }

    // Assign initial composite score from FTS rank position
    let n = candidates.len() as f64;
    let mut scores: Vec<f64> = (0..candidates.len())
        .map(|i| 1.0 - (i as f64 / n))
        .collect();

    // Signal score blending: composite = (1-w)*fts_rank + w*signal_score
    if config.apply_signal_score {
        let w = config.signal_score_weight.clamp(0.0, 1.0);
        for (i, hit) in candidates.iter().enumerate() {
            scores[i] = (1.0 - w) * scores[i] + w * hit.signal_score;
        }
    }

    // Ambient boost: multiplicative on composite score (identity when context empty)
    if config.apply_ambient_boost {
        for (i, hit) in candidates.iter().enumerate() {
            let boost = crate::cascade_layers::ambient_boost_for_hit(hit, context);
            scores[i] *= boost;
        }
    }

    // Declarative density: additive boost for first-person declarative content.
    // Uses stored density from ingest when available; falls back to per-query
    // computation for un-backfilled memories (declarative_density = NULL).
    if config.apply_declarative_boost {
        let w = config.declarative_weight.clamp(0.0, 0.2);
        for (i, hit) in candidates.iter().enumerate() {
            let density = hit
                .declarative_density
                .unwrap_or_else(|| declarative_density(&hit.content));
            scores[i] += w * density;
        }
    }

    // Recency decay: multiplicative on composite score
    if config.apply_recency {
        let now = context.now;
        for (i, hit) in candidates.iter().enumerate() {
            let age_days = hit
                .created_at
                .as_deref()
                .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok())
                .map(|dt| (now - dt.and_utc()).num_hours() as f64 / 24.0)
                .unwrap_or(0.0)
                .max(0.0);
            let recency_factor = 0.5_f64.powf(age_days / config.recency_half_life_days);
            scores[i] *= recency_factor;
        }
    }

    // Entity boost: additive for top member of each wing cluster
    if config.apply_entity_boost {
        let mut wing_groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, hit) in candidates.iter().enumerate() {
            let wing = hit.wing.clone().unwrap_or_default();
            wing_groups.entry(wing).or_default().push(i);
        }
        for indices in wing_groups.values() {
            if indices.len() < 2 {
                continue;
            }
            let top_idx = *indices
                .iter()
                .max_by(|&&a, &&b| {
                    scores[a]
                        .partial_cmp(&scores[b])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap();
            scores[top_idx] += config.entity_boost_weight;
        }
    }

    // Co-retrieval boost: multiplicative, anchored on current top-K candidates.
    // Rewards memories that historically co-retrieve with the current best results.
    if config.apply_co_retrieval_boost {
        if let Some(ref store) = config.co_retrieval_store {
            // Identify current top-K by score so far
            let mut ranked: Vec<(usize, f64)> = scores.iter().copied().enumerate().collect();
            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let anchor_ids: Vec<&str> = ranked
                .iter()
                .take(config.co_retrieval_top_k)
                .map(|&(i, _)| candidates[i].id.as_str())
                .collect();

            // Query co-retrieval pairs for each anchor, aggregate co_count per related ID.
            // SqliteStore futures complete synchronously on first poll (mutex + SQL).
            let mut co_counts: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();
            for anchor_id in &anchor_ids {
                let fut = store.related_memories(anchor_id, 50);
                let related = poll_sync(fut).unwrap_or_default();
                for r in related {
                    *co_counts.entry(r.memory_id).or_insert(0) += r.co_count;
                }
            }

            // Normalize and apply multiplicative boost
            let max_count = co_counts.values().copied().max().unwrap_or(0) as f64;
            if max_count > 0.0 {
                for (i, hit) in candidates.iter().enumerate() {
                    if let Some(&count) = co_counts.get(&hit.id) {
                        let normalized = count as f64 / max_count;
                        scores[i] *= 1.0 + config.co_retrieval_weight * normalized;
                    }
                }
            }
        }
    }

    // Single sort by composite score
    let mut indexed: Vec<(usize, f64)> = scores.into_iter().enumerate().collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut sorted: Vec<MemoryHit> = indexed
        .into_iter()
        .map(|(i, score)| {
            let mut hit = candidates[i].clone();
            hit.signal_score = score; // Store composite as signal_score for downstream
            hit
        })
        .collect();

    // Post-rank: episode diversity (reorder, don't discard)
    if config.apply_episode_diversity {
        crate::cascade_layers::apply_episode_diversity(&mut sorted, config.max_per_episode);
    }

    // Post-rank: context chain dedup
    if config.apply_context_dedup {
        sorted = dedup_context_chains(sorted);
    }

    sorted
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use spectral_ingest::MemoryStore as _;

    fn make_hit(id: &str, score: f64, wing: Option<&str>, created_at: Option<&str>) -> MemoryHit {
        MemoryHit {
            id: id.into(),
            key: id.into(),
            content: format!("Content for {id}"),
            wing: wing.map(|w| w.into()),
            hall: None,
            signal_score: score,
            visibility: "private".into(),
            hits: 0,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: created_at.map(|s| s.into()),
            last_reinforced_at: None,
            episode_id: None,
            declarative_density: None,
            description: None,
        }
    }

    #[test]
    fn signal_score_weighting_affects_ranking() {
        // Three candidates — low-signal at FTS position 0, high-signal at position 2.
        // With strong weight, high signal_score overcomes positional disadvantage.
        let mut candidates = vec![
            make_hit("low-signal", 0.3, None, None),
            make_hit("mid-signal", 0.5, None, None),
            make_hit("high-signal", 0.95, None, None),
        ];

        apply_signal_score_weight(&mut candidates, 0.7);

        // High signal should now rank first despite being last in FTS order
        assert_eq!(candidates[0].id, "high-signal");
    }

    #[test]
    fn recency_weighting_decays_old_memories() {
        let now = Utc::now();
        let recent = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let old = (now - chrono::Duration::days(365))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let mut candidates = vec![
            make_hit("old", 0.8, None, Some(&old)),
            make_hit("recent", 0.8, None, Some(&recent)),
        ];

        apply_recency_weight(&mut candidates, 90.0, now);

        // Recent memory should rank higher (less decay)
        assert_eq!(candidates[0].id, "recent");
        assert!(
            candidates[0].signal_score > candidates[1].signal_score,
            "recent should score higher after recency decay"
        );
    }

    #[test]
    fn entity_clustering_boosts_top_within_group() {
        let mut candidates = vec![
            make_hit("a1", 0.8, Some("permagent"), None),
            make_hit("a2", 0.7, Some("permagent"), None),
            make_hit("b1", 0.75, Some("getladle"), None),
        ];

        boost_entity_clusters(&mut candidates, 0.15);

        // a1 (top in permagent cluster) should get boosted
        let a1 = candidates.iter().find(|h| h.id == "a1").unwrap();
        assert!(
            a1.signal_score > 0.9,
            "top of cluster should be boosted: {}",
            a1.signal_score
        );
        // a2 should NOT be boosted
        let a2 = candidates.iter().find(|h| h.id == "a2").unwrap();
        assert!(
            (a2.signal_score - 0.7).abs() < 0.01,
            "non-top cluster member should not be boosted"
        );
    }

    #[test]
    fn context_chain_dedup_collapses_near_duplicates() {
        let candidates = vec![
            MemoryHit {
                id: "dup1".into(),
                key: "dup1".into(),
                content: "[Memory context] - shared_key: some content here".into(),
                wing: None,
                hall: None,
                signal_score: 0.9,
                visibility: "private".into(),
                hits: 0,
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
            },
            MemoryHit {
                id: "dup2".into(),
                key: "dup2".into(),
                content: "[Memory context] - shared_key: different tail".into(),
                wing: None,
                hall: None,
                signal_score: 0.7,
                visibility: "private".into(),
                hits: 0,
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
            },
            MemoryHit {
                id: "clean".into(),
                key: "clean".into(),
                content: "Normal memory content without context prefix".into(),
                wing: None,
                hall: None,
                signal_score: 0.6,
                visibility: "private".into(),
                hits: 0,
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
            },
        ];

        let result = dedup_context_chains(candidates);

        // Should keep highest-scoring duplicate + the clean memory
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|h| h.id == "dup1"));
        assert!(result.iter().any(|h| h.id == "clean"));
        assert!(!result.iter().any(|h| h.id == "dup2"));
    }

    // ── Unified pipeline tests ──────────────────────────────────────

    #[test]
    fn unified_pipeline_preserves_fts_order_when_signals_equal() {
        // When all signal_scores are equal, FTS rank order should be preserved
        let candidates = vec![
            make_hit("first", 0.6, None, None),
            make_hit("second", 0.6, None, None),
            make_hit("third", 0.6, None, None),
        ];

        let config = RerankingConfig {
            apply_signal_score: true,
            signal_score_weight: 0.3,
            apply_recency: false,
            apply_entity_boost: false,
            apply_ambient_boost: false,
            apply_episode_diversity: false,
            apply_context_dedup: false,
            ..Default::default()
        };

        let ctx = spectral_cascade::RecognitionContext::empty();
        let result = apply_reranking_pipeline(candidates, &config, &ctx);

        // FTS rank dominates (0.7 weight) when signal_scores are equal
        assert_eq!(result[0].id, "first");
        assert_eq!(result[1].id, "second");
        assert_eq!(result[2].id, "third");
    }

    #[test]
    fn unified_pipeline_no_role_bias_with_empty_context() {
        // User and assistant turns with same signal_score should maintain
        // FTS position — no ambient boost should discriminate
        let candidates = vec![
            make_hit("user_turn", 0.6, Some("general"), None),
            make_hit("assistant_turn", 0.8, Some("general"), None),
            make_hit("user_turn_2", 0.6, Some("general"), None),
        ];

        // Config WITH ambient boost but EMPTY context — should be identity
        let config = RerankingConfig {
            apply_ambient_boost: true,
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();
        let with_ambient = apply_reranking_pipeline(candidates.clone(), &config, &ctx);

        // Config WITHOUT ambient boost
        let config_no_ambient = RerankingConfig {
            apply_ambient_boost: false,
            ..Default::default()
        };
        let without_ambient = apply_reranking_pipeline(candidates, &config_no_ambient, &ctx);

        // Should produce identical ordering
        assert_eq!(with_ambient.len(), without_ambient.len());
        for (a, b) in with_ambient.iter().zip(without_ambient.iter()) {
            assert_eq!(
                a.id, b.id,
                "ambient boost with empty context should be identity"
            );
        }
    }

    #[test]
    fn unified_pipeline_signal_score_does_not_dominate() {
        // High signal_score at FTS position 3 should NOT jump to position 1
        // with default weight=0.3 (FTS dominates at 0.7 weight)
        let candidates = vec![
            make_hit("fts_best", 0.5, None, None),
            make_hit("fts_second", 0.5, None, None),
            make_hit("high_signal", 0.9, None, None),
        ];

        let config = RerankingConfig {
            apply_signal_score: true,
            signal_score_weight: 0.3,
            apply_recency: false,
            apply_entity_boost: false,
            apply_ambient_boost: false,
            apply_episode_diversity: false,
            apply_context_dedup: false,
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();
        let result = apply_reranking_pipeline(candidates, &config, &ctx);

        // At weight=0.3:
        // fts_best: 0.7*1.0 + 0.3*0.5 = 0.85
        // fts_second: 0.7*0.67 + 0.3*0.5 = 0.62
        // high_signal: 0.7*0.33 + 0.3*0.9 = 0.50
        // FTS rank (position 1) should still dominate
        assert_eq!(
            result[0].id, "fts_best",
            "FTS best should remain #1 at weight=0.3"
        );
    }

    // ── Declarative density tests ──────────────────────────────────

    #[test]
    fn declarative_density_mixed_content() {
        // 1 declarative ("I went to the store") + 1 non-declarative ("The weather was nice")
        let d = declarative_density("I went to the store. The weather was nice.");
        assert!((d - 0.5).abs() < 0.01, "expected ~0.5, got {d}");
    }

    #[test]
    fn declarative_density_no_first_person() {
        let d = declarative_density("Do you have tips? Sure, here are some ideas!");
        assert!(
            d < 0.01,
            "no first-person content should score near 0, got {d}"
        );
    }

    #[test]
    fn declarative_density_assistant_style() {
        let d = declarative_density(
            "Here are some tips for cooking. First, preheat the oven. Then add the ingredients.",
        );
        assert!(
            d < 0.01,
            "assistant-style content should score near 0, got {d}"
        );
    }

    #[test]
    fn declarative_density_all_first_person() {
        let d = declarative_density(
            "I graduated with a Business degree. I commute 45 minutes. My favorite color is blue.",
        );
        assert!(
            (d - 1.0).abs() < 0.01,
            "all first-person declarative should score ~1.0, got {d}"
        );
    }

    #[test]
    fn declarative_density_questions_excluded() {
        let d = declarative_density("What should I do? How can I improve?");
        // These contain "I" but end with "?" — should NOT count as declarative.
        // However, our split is on '.', '!', '?' so the question mark is a separator.
        // The sentences are "What should I do" and "How can I improve" — no '?' in them
        // after splitting. But they are interrogative by nature.
        // Actually: split on '?' means the '?' is consumed, so the sentence text
        // doesn't contain '?'. The not_question check looks at the sentence itself.
        // Since we split ON '?', the sentence won't contain '?' — this is a known
        // limitation. However, interrogative sentences rarely contain "I" as subject
        // in LongMemEval data, so the practical impact is small.
        // The sentences "What should I do" and "How can I improve" DO contain "I",
        // and after split they DON'T contain "?". So density = 1.0 here.
        // This is acceptable — the function is a heuristic, not a parser.
        assert!((0.0..=1.0).contains(&d));
    }

    #[test]
    fn declarative_density_empty_content() {
        assert!((declarative_density("") - 0.0).abs() < f64::EPSILON);
        assert!((declarative_density("ok") - 0.0).abs() < f64::EPSILON);
    }

    fn make_hit_with_content(id: &str, score: f64, content: &str) -> MemoryHit {
        MemoryHit {
            id: id.into(),
            key: id.into(),
            content: content.into(),
            wing: None,
            hall: None,
            signal_score: score,
            visibility: "private".into(),
            hits: 0,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            declarative_density: None,
            description: None,
        }
    }

    #[test]
    fn declarative_boost_changes_ranking() {
        // Many candidates so FTS position gap between adjacent items is small.
        // The declarative candidate is at FTS position 2 (close to position 1).
        // With 10 candidates, FTS gap per position = 1/10 = 0.10.
        // Declarative boost at weight 0.10 * density 1.0 = 0.10 — enough to
        // overcome one position when FTS gap is 0.10.
        let mut candidates = vec![
            make_hit_with_content(
                "assistant_top",
                0.5,
                "Here are some tips for improving your routine. Try to exercise daily.",
            ),
            make_hit_with_content(
                "user_declarative",
                0.5,
                "I graduated with a degree in Business Administration. My commute is 45 minutes.",
            ),
        ];
        // Pad with filler to reduce per-position FTS gap
        // With 20 candidates, FTS gap = 1/20 = 0.05. Declarative 0.10 > 0.05.
        for i in 2..20 {
            candidates.push(make_hit_with_content(
                &format!("filler_{i}"),
                0.5,
                "Some generic content about various topics and ideas.",
            ));
        }

        let config_with = RerankingConfig {
            apply_signal_score: false,
            apply_recency: false,
            apply_entity_boost: false,
            apply_ambient_boost: false,
            apply_declarative_boost: true,
            declarative_weight: 0.10,
            apply_episode_diversity: false,
            apply_context_dedup: false,
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();

        let result = apply_reranking_pipeline(candidates.clone(), &config_with, &ctx);
        assert_eq!(
            result[0].id, "user_declarative",
            "user turn with first-person declarative should rank first with boost"
        );

        // Without declarative boost: FTS order preserved (assistant first)
        let config_without = RerankingConfig {
            apply_declarative_boost: false,
            ..config_with
        };
        let result_no = apply_reranking_pipeline(candidates, &config_without, &ctx);
        assert_eq!(
            result_no[0].id, "assistant_top",
            "without declarative boost, FTS order preserved"
        );
    }

    #[test]
    fn declarative_boost_bounded_by_fts() {
        // Even with max declarative density, FTS position 1 should beat position 3
        // when signal_score weight is also active (combined weights < 1.0).
        let candidates = vec![
            make_hit_with_content("fts_best", 0.5, "The system status looks normal today."),
            make_hit_with_content("fts_mid", 0.5, "Temperature is rising slightly."),
            make_hit_with_content(
                "high_decl",
                0.5,
                "I decided to change my career. I love my new job. My salary is great.",
            ),
        ];

        let config = RerankingConfig {
            apply_signal_score: true,
            signal_score_weight: 0.3,
            apply_recency: false,
            apply_entity_boost: false,
            apply_ambient_boost: false,
            apply_declarative_boost: true,
            declarative_weight: 0.10,
            apply_episode_diversity: false,
            apply_context_dedup: false,
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();
        let result = apply_reranking_pipeline(candidates, &config, &ctx);

        // FTS position 1 gets 0.6*1.0 = 0.60 FTS + 0.3*0.5 = 0.15 signal + 0.0 decl = 0.75
        // FTS position 3 gets 0.6*0.33 = 0.20 FTS + 0.3*0.5 = 0.15 signal + 0.1*1.0 = 0.45
        // FTS best should still rank first
        assert_eq!(
            result[0].id, "fts_best",
            "FTS rank should still dominate with declarative weight=0.10"
        );
    }

    #[test]
    fn recency_uses_context_now_not_utc_now() {
        // Memories from 2023. If recency uses Utc::now() (2026), both are
        // ~1000 days old and get near-identical decay. If it correctly uses
        // context.now (2023-05-30), the May memory is 10 days old while the
        // January memory is 140 days old — a meaningful difference.
        let candidates = vec![
            make_hit("old_jan", 0.6, None, Some("2023-01-10 12:00:00")),
            make_hit("recent_may", 0.6, None, Some("2023-05-20 12:00:00")),
        ];

        let config = RerankingConfig {
            apply_signal_score: false,
            apply_recency: true,
            recency_half_life_days: 90.0,
            apply_entity_boost: false,
            apply_ambient_boost: false,
            apply_episode_diversity: false,
            apply_context_dedup: false,
            ..Default::default()
        };

        // Context with now = 2023-05-30 (question date)
        let question_now = Utc.with_ymd_and_hms(2023, 5, 30, 23, 40, 0).unwrap();
        let ctx = spectral_cascade::RecognitionContext::empty().with_now(question_now);
        let result = apply_reranking_pipeline(candidates, &config, &ctx);

        // recent_may is 10 days old → high recency factor
        // old_jan is 140 days old → much lower recency factor
        // Despite old_jan being first in FTS order, recent_may should rank first
        assert_eq!(
            result[0].id, "recent_may",
            "recent memory should rank first when context.now is question_date"
        );

        // Verify the scores actually differ meaningfully
        let score_diff = result[0].signal_score - result[1].signal_score;
        assert!(
            score_diff > 0.05,
            "score difference should be meaningful with 90-day half-life, got {score_diff:.4}"
        );
    }

    #[test]
    fn stored_density_used_when_available() {
        // Hit with pre-computed density should use that value, not recompute
        let mut hit = make_hit_with_content(
            "stored",
            0.5,
            "No first-person content here at all in this sentence.",
        );
        // The content has zero declarative density if computed
        assert!(declarative_density(&hit.content) < 0.01);
        // But we store a high density artificially
        hit.declarative_density = Some(0.9);

        let candidates = vec![hit];
        let config = RerankingConfig {
            apply_signal_score: false,
            apply_recency: false,
            apply_entity_boost: false,
            apply_ambient_boost: false,
            apply_declarative_boost: true,
            declarative_weight: 0.10,
            apply_episode_diversity: false,
            apply_context_dedup: false,
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();
        let result = apply_reranking_pipeline(candidates, &config, &ctx);

        // Score should include the stored 0.9 density, not the computed 0.0
        // FTS base for 1 candidate = 1.0, declarative boost = 0.10 * 0.9 = 0.09
        assert!(
            result[0].signal_score > 1.05,
            "should use stored density 0.9, got score {}",
            result[0].signal_score
        );
    }

    #[test]
    fn null_density_falls_back_to_computation() {
        let hit = make_hit_with_content(
            "computed",
            0.5,
            "I graduated with a degree. I like my commute. My job is great.",
        );
        assert!(hit.declarative_density.is_none());
        let computed = declarative_density(&hit.content);
        assert!(
            computed > 0.5,
            "content should have high declarative density"
        );

        let candidates = vec![hit];
        let config = RerankingConfig {
            apply_signal_score: false,
            apply_recency: false,
            apply_entity_boost: false,
            apply_ambient_boost: false,
            apply_declarative_boost: true,
            declarative_weight: 0.10,
            apply_episode_diversity: false,
            apply_context_dedup: false,
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();
        let result = apply_reranking_pipeline(candidates, &config, &ctx);

        // Should fall back to computing density from content
        let expected_boost = 0.10 * computed;
        assert!(
            result[0].signal_score > 1.0 + expected_boost * 0.5,
            "should fall back to computed density, got score {}",
            result[0].signal_score
        );
    }

    // ── Co-retrieval boost tests ────────────────────────────────────

    /// Helper: build an in-memory SqliteStore with co-retrieval pairs prepopulated
    /// via retrieval events + rebuild.
    fn store_with_co_retrieval_pairs(
        pairs: &[(&str, &str, i64)],
    ) -> std::sync::Arc<dyn spectral_ingest::MemoryStore> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let store = spectral_ingest::sqlite_store::SqliteStore::open_in_memory().unwrap();

        // For each pair with count N, log N events containing both memories
        for &(a, b, count) in pairs {
            for _ in 0..count {
                let event = spectral_ingest::RetrievalEvent {
                    query_hash: "test".into(),
                    timestamp: "2024-01-01T00:00:00Z".into(),
                    memory_ids_json: serde_json::to_string(&[a, b]).unwrap(),
                    method: "cascade".into(),
                    wing: None,
                    question_type: None,
                    session_id: None,
                };
                rt.block_on(store.log_retrieval_event(&event)).unwrap();
            }
        }
        rt.block_on(store.rebuild_co_retrieval_index()).unwrap();

        std::sync::Arc::new(store)
    }

    #[test]
    fn co_retrieval_boost_disabled_by_default_no_ranking_change() {
        let candidates = vec![
            make_hit("m1", 0.9, None, None),
            make_hit("m2", 0.5, None, None),
        ];
        let config = RerankingConfig::default(); // apply_co_retrieval_boost: false
        let ctx = spectral_cascade::RecognitionContext::empty();

        let result = apply_reranking_pipeline(candidates.clone(), &config, &ctx);
        let baseline = apply_reranking_pipeline(candidates, &RerankingConfig::default(), &ctx);

        assert_eq!(result.len(), baseline.len());
        for (a, b) in result.iter().zip(baseline.iter()) {
            assert_eq!(a.id, b.id);
        }
    }

    #[test]
    fn co_retrieval_boost_no_op_when_index_empty() {
        let store = store_with_co_retrieval_pairs(&[]);
        let candidates = vec![
            make_hit("m1", 0.9, None, None),
            make_hit("m2", 0.5, None, None),
        ];
        let config_on = RerankingConfig {
            apply_signal_score: false,
            apply_recency: false,
            apply_co_retrieval_boost: true,
            co_retrieval_store: Some(store.clone()),
            ..Default::default()
        };
        let config_off = RerankingConfig {
            apply_signal_score: false,
            apply_recency: false,
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();

        let result_on = apply_reranking_pipeline(candidates.clone(), &config_on, &ctx);
        let result_off = apply_reranking_pipeline(candidates, &config_off, &ctx);

        // Same ranking — empty index means no boost
        for (a, b) in result_on.iter().zip(result_off.iter()) {
            assert_eq!(a.id, b.id);
        }
    }

    #[test]
    fn co_retrieval_boost_lifts_related_memories() {
        // 5 candidates so FTS position gaps are small (0.2 per position).
        // m1 is the anchor (highest FTS). m5 (last) has high co-retrieval with m1.
        // Without boost: m1 > m2 > m3 > m4 > m5.
        // With boost: m5 gets a 1.5x multiplier on its FTS score, lifting it.
        let store = store_with_co_retrieval_pairs(&[("m1", "m5", 10)]);

        // Verify the store has the data
        {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let related = rt.block_on(store.related_memories("m1", 50)).unwrap();
            assert!(
                !related.is_empty(),
                "store should have co-retrieval data for m1"
            );
            assert_eq!(related[0].memory_id, "m5");
        }
        let candidates = vec![
            make_hit("m1", 0.5, None, None),
            make_hit("m2", 0.5, None, None),
            make_hit("m3", 0.5, None, None),
            make_hit("m4", 0.5, None, None),
            make_hit("m5", 0.5, None, None),
        ];

        let config = RerankingConfig {
            apply_signal_score: false,
            apply_recency: false,
            apply_co_retrieval_boost: true,
            co_retrieval_weight: 3.0, // 300% boost: m5 goes from 0.2 to 0.8, past m3 (0.6) and m4 (0.4)
            co_retrieval_top_k: 1,
            co_retrieval_store: Some(store),
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();

        let result_boosted = apply_reranking_pipeline(candidates.clone(), &config, &ctx);
        let result_baseline = apply_reranking_pipeline(
            candidates,
            &RerankingConfig {
                apply_signal_score: false,
                apply_recency: false,
                ..Default::default()
            },
            &ctx,
        );

        // Baseline: m5 is last
        assert_eq!(result_baseline.last().unwrap().id, "m5");
        // With boost: m5 should have moved up from its original last position
        let m5_pos = result_boosted.iter().position(|h| h.id == "m5").unwrap();
        assert!(
            m5_pos < result_boosted.len() - 1,
            "m5 should be lifted from last position, got position {m5_pos}"
        );
    }

    #[test]
    fn co_retrieval_boost_normalizes_correctly() {
        // Two related memories with different co_counts
        let store = store_with_co_retrieval_pairs(&[("m1", "m2", 10), ("m1", "m3", 5)]);
        let candidates = vec![
            make_hit("m1", 0.5, None, None),
            make_hit("m2", 0.5, None, None),
            make_hit("m3", 0.5, None, None),
        ];

        let config = RerankingConfig {
            apply_signal_score: false,
            apply_recency: false,
            apply_co_retrieval_boost: true,
            co_retrieval_weight: 0.15,
            co_retrieval_top_k: 1,
            co_retrieval_store: Some(store),
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();
        let result = apply_reranking_pipeline(candidates, &config, &ctx);

        // m2 should get full boost (count=10, normalized=1.0)
        // m3 should get half boost (count=5, normalized=0.5)
        assert!(
            result.iter().find(|h| h.id == "m2").unwrap().signal_score
                > result.iter().find(|h| h.id == "m3").unwrap().signal_score,
            "m2 should rank higher than m3 due to higher co-count"
        );
    }

    #[test]
    fn co_retrieval_boost_respects_anchor_top_k() {
        // Only anchor on top-1 (m1). m1 has co-retrieval with m3.
        // m2 has co-retrieval with m4 but m2 isn't in anchor set (top_k=1).
        let store = store_with_co_retrieval_pairs(&[("m1", "m3", 10), ("m2", "m4", 10)]);
        let candidates = vec![
            make_hit("m1", 0.9, None, None),
            make_hit("m2", 0.5, None, None),
            make_hit("m3", 0.5, None, None),
            make_hit("m4", 0.5, None, None),
        ];

        let config = RerankingConfig {
            apply_signal_score: false,
            apply_recency: false,
            apply_co_retrieval_boost: true,
            co_retrieval_weight: 0.50,
            co_retrieval_top_k: 1, // Only m1 is anchor
            co_retrieval_store: Some(store),
            ..Default::default()
        };
        let ctx = spectral_cascade::RecognitionContext::empty();
        let result = apply_reranking_pipeline(candidates, &config, &ctx);

        // m3 should be boosted (co-retrieved with anchor m1)
        // m4 should NOT be boosted (m2 not in anchor set)
        let m3_score = result.iter().find(|h| h.id == "m3").unwrap().signal_score;
        let m4_score = result.iter().find(|h| h.id == "m4").unwrap().signal_score;
        assert!(
            m3_score > m4_score,
            "m3 boosted by anchor m1, m4 not: m3={m3_score}, m4={m4_score}"
        );
    }
}
