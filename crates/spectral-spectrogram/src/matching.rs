//! Cross-wing resonance matching.
//!
//! Given a query memory's spectral fingerprint, finds memories in OTHER wings
//! whose fingerprints are "harmonically similar" — same action_type and
//! comparable values on multiple dimensions within configurable tolerances.

use crate::types::SpectralFingerprint;

/// A match found by cross-wing resonance search.
#[derive(Debug, Clone)]
pub struct ResonantMatch {
    pub memory_id: String,
    /// Overall resonance score, 0.0 to 1.0.
    pub resonance_score: f64,
    /// Which dimensions matched within tolerance.
    pub matched_dimensions: Vec<String>,
}

/// Tolerance thresholds for dimension matching.
#[derive(Debug, Clone)]
pub struct MatchTolerances {
    pub entity_density: f64,
    pub decision_polarity: f64,
    pub causal_depth: f64,
    pub emotional_valence: f64,
    pub temporal_specificity: f64,
    pub novelty: f64,
    /// Minimum number of dimensions that must match (besides action_type).
    pub min_matching_dimensions: usize,
}

impl Default for MatchTolerances {
    fn default() -> Self {
        Self {
            entity_density: 0.3,
            decision_polarity: 0.4,
            causal_depth: 0.3,
            emotional_valence: 0.4,
            temporal_specificity: 0.3,
            novelty: 0.3,
            min_matching_dimensions: 3,
        }
    }
}

/// Find memories whose spectral fingerprints resonate with the query fingerprint.
///
/// Resonance requires matching action_type plus at least `min_matching_dimensions`
/// numeric dimensions within the configured tolerances.
pub fn find_resonant(
    query: &SpectralFingerprint,
    candidates: &[SpectralFingerprint],
    max_results: usize,
    tolerances: &MatchTolerances,
) -> Vec<ResonantMatch> {
    let mut matches: Vec<ResonantMatch> = candidates
        .iter()
        .filter(|c| c.memory_id != query.memory_id)
        .filter(|c| c.action_type == query.action_type)
        .filter_map(|c| {
            let mut matched = Vec::new();

            if (c.entity_density - query.entity_density).abs() <= tolerances.entity_density {
                matched.push("entity_density".to_string());
            }
            if (c.decision_polarity - query.decision_polarity).abs() <= tolerances.decision_polarity
            {
                matched.push("decision_polarity".to_string());
            }
            if (c.causal_depth - query.causal_depth).abs() <= tolerances.causal_depth {
                matched.push("causal_depth".to_string());
            }
            if (c.emotional_valence - query.emotional_valence).abs() <= tolerances.emotional_valence
            {
                matched.push("emotional_valence".to_string());
            }
            if (c.temporal_specificity - query.temporal_specificity).abs()
                <= tolerances.temporal_specificity
            {
                matched.push("temporal_specificity".to_string());
            }
            if (c.novelty - query.novelty).abs() <= tolerances.novelty {
                matched.push("novelty".to_string());
            }

            if matched.len() >= tolerances.min_matching_dimensions {
                let resonance_score = matched.len() as f64 / 6.0;
                Some(ResonantMatch {
                    memory_id: c.memory_id.clone(),
                    resonance_score,
                    matched_dimensions: matched,
                })
            } else {
                None
            }
        })
        .collect();

    matches.sort_by(|a, b| {
        b.resonance_score
            .partial_cmp(&a.resonance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matches.truncate(max_results);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ActionType;
    use chrono::Utc;

    fn make_fp(
        id: &str,
        action: ActionType,
        ed: f64,
        ev: f64,
        cd: f64,
        nv: f64,
    ) -> SpectralFingerprint {
        SpectralFingerprint {
            memory_id: id.into(),
            entity_density: ed,
            action_type: action,
            decision_polarity: 0.0,
            causal_depth: cd,
            emotional_valence: ev,
            temporal_specificity: 0.0,
            novelty: nv,
            peak_dimensions: vec![],
            created_at: Utc::now(),
        }
    }

    #[test]
    fn matching_finds_resonance() {
        let query = make_fp("q", ActionType::Decision, 0.5, 0.3, 0.4, 0.5);
        let candidates = vec![
            make_fp("c1", ActionType::Decision, 0.5, 0.3, 0.4, 0.5), // should match
            make_fp("c2", ActionType::Task, 0.5, 0.3, 0.4, 0.5),     // wrong action_type
            make_fp("c3", ActionType::Decision, 0.99, 0.99, 0.99, 0.99), // too different
        ];
        let strict = MatchTolerances {
            entity_density: 0.1,
            emotional_valence: 0.1,
            causal_depth: 0.1,
            novelty: 0.1,
            ..MatchTolerances::default()
        };
        let results = find_resonant(&query, &candidates, 10, &strict);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].memory_id, "c1");
    }

    #[test]
    fn matching_excludes_self() {
        let query = make_fp("q", ActionType::Decision, 0.5, 0.3, 0.4, 0.5);
        let candidates = vec![make_fp("q", ActionType::Decision, 0.5, 0.3, 0.4, 0.5)];
        let results = find_resonant(&query, &candidates, 10, &MatchTolerances::default());
        assert!(results.is_empty());
    }
}
