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

/// Configuration for the unified re-ranking pipeline.
/// Both topk_fts and cascade call this with different configs.
#[derive(Debug, Clone)]
pub struct RerankingConfig {
    pub apply_signal_score: bool,
    pub signal_score_weight: f64,
    pub apply_recency: bool,
    pub recency_half_life_days: f64,
    pub apply_entity_boost: bool,
    pub entity_boost_weight: f64,
    pub apply_ambient_boost: bool,
    pub apply_episode_diversity: bool,
    pub max_per_episode: usize,
    pub apply_context_dedup: bool,
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
            apply_episode_diversity: false,
            max_per_episode: 5,
            apply_context_dedup: true,
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
}
