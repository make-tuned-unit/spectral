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

#[cfg(test)]
mod tests {
    use super::*;

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
}
