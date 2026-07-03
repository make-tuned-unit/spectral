//! Scoring: rarity-weighted evidence (REM) + global echo (MINERVA 2).
//!
//! REM's insight: the diagnostic value of a matched feature is its rarity —
//! matching "exit-137~OOMKilled" in one memory of a thousand is strong
//! evidence of "old"; matching "weekly~report" that occurs everywhere is
//! weak. Evidence weight = ln(enrolled / doc_frequency), summed per trace.
//!
//! MINERVA 2's insight: familiarity is a property of the whole memory, not
//! one trace — echo intensity is the sum of *cubed* similarities to every
//! stored trace, so many weak resonances stay quiet while one strong match
//! rings. We normalize per-trace coverage to [0,1] and cube it.

use crate::store::FeatureMatch;
use crate::{Evidence, RecognitionResult, StimulusPrints, TraceMatch, Verdict};
use std::collections::HashMap;

/// Thresholds and weights for verdict formation.
#[derive(Debug, Clone)]
pub struct ScoreConfig {
    /// Minimum coverage (fraction of stimulus fingerprints matched by one
    /// trace) for a `Recognized` verdict.
    pub recognize_coverage: f64,
    /// Minimum rarity-weighted score for a `Recognized` verdict (guards
    /// against tiny stimuli where coverage is trivially high).
    pub recognize_min_score: f64,
    /// Lead margin: best trace must exceed runner-up's score by this factor
    /// to be `Recognized` (ACR's θ+δ margin rule; prevents flapping between
    /// two similar traces).
    pub recognize_margin: f64,
    /// Familiarity floor for a `Familiar` verdict.
    pub familiar_floor: f64,
    /// Alternative Familiar path: best-trace rarity-weighted score at or
    /// above this triggers Familiar even at low coverage (REM: a couple of
    /// very rare matched anchors are strong evidence despite covering
    /// little of the stimulus).
    pub familiar_min_score: f64,
    /// Winnowed-gram hits weigh this multiple of an equally-rare pair hit
    /// (verbatim runs are stronger identity evidence than co-occurrence).
    pub gram_weight: f64,
    /// Maximum evidence rows returned (strongest first).
    pub max_evidence: usize,
    /// Maximum candidate traces returned.
    pub max_traces: usize,
}

impl Default for ScoreConfig {
    fn default() -> Self {
        Self {
            recognize_coverage: 0.35,
            recognize_min_score: 3.0,
            recognize_margin: 1.5,
            familiar_floor: 0.10,
            familiar_min_score: 2.5,
            gram_weight: 2.0,
            max_evidence: 12,
            max_traces: 5,
        }
    }
}

struct Accum {
    score: f64,
    pair_hits: usize,
    gram_hits: usize,
    evidence: Vec<Evidence>,
}

/// Rarity weight for a feature: ln((enrolled + 1) / doc_frequency).
/// +1 smooths the tiny-corpus case; df >= 1 whenever a match exists.
fn rarity(enrolled: usize, df: usize) -> f64 {
    (((enrolled + 1) as f64) / (df.max(1) as f64)).ln().max(0.1)
}

/// Score candidates and form a verdict.
pub fn score_candidates(
    prints: &StimulusPrints,
    pair_matches: &[FeatureMatch],
    gram_matches: &[FeatureMatch],
    enrolled: usize,
    config: &ScoreConfig,
) -> RecognitionResult {
    let stimulus_features = prints.pair_hashes.len() + prints.gram_hashes.len();
    let mut acc: HashMap<String, Accum> = HashMap::new();

    for (matches, is_gram) in [(pair_matches, false), (gram_matches, true)] {
        for m in matches {
            let base = rarity(enrolled, m.doc_frequency);
            let w = if is_gram {
                base * config.gram_weight
            } else {
                base
            };
            let a = acc.entry(m.memory_id.clone()).or_insert(Accum {
                score: 0.0,
                pair_hits: 0,
                gram_hits: 0,
                evidence: Vec::new(),
            });
            a.score += w;
            if is_gram {
                a.gram_hits += 1;
            } else {
                a.pair_hits += 1;
            }
            a.evidence.push(Evidence {
                feature: m.label.clone(),
                memory_id: m.memory_id.clone(),
                weight: w,
            });
        }
    }

    // Build trace list, strongest first. Deterministic tie-break by id.
    let mut traces: Vec<TraceMatch> = acc
        .iter()
        .map(|(id, a)| TraceMatch {
            memory_id: id.clone(),
            score: a.score,
            pair_hits: a.pair_hits,
            gram_hits: a.gram_hits,
            coverage: if stimulus_features > 0 {
                // Distinct feature hits capped at stimulus feature count —
                // a trace can't cover more than the stimulus has.
                ((a.pair_hits + a.gram_hits) as f64 / stimulus_features as f64).min(1.0)
            } else {
                0.0
            },
        })
        .collect();
    traces.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.memory_id.cmp(&b.memory_id))
    });

    // Familiarity = best-trace rarity-weighted coverage: matched evidence
    // weight over the stimulus's total POTENTIAL evidence weight, where an
    // unmatched feature counts at maximum rarity (df=1). A degraded true
    // re-encounter matches most of its rare features -> near 1; a topical
    // near-miss matches a few common features out of many -> near 0.
    // (REM's likelihood structure with MINERVA's whole-memory framing.)
    let max_weight = rarity(enrolled, 1);
    let n_pair = prints.pair_hashes.len() as f64;
    let n_gram = prints.gram_hashes.len() as f64;
    let total_potential = max_weight * (n_pair + config.gram_weight * n_gram);
    // NOTE on scalar scope (measured, 2026-07-02): coverage familiarity
    // separates DEGRADED re-encounters cleanly (AUC 0.95 on real data) but
    // NOT paraphrases (AUC ~0.55) — paraphrases share few features with
    // their source. Blending in absolute evidence (score/(score+k)) was
    // tried and REJECTED: it lifted topical negatives more than paraphrase
    // positives (degraded AUC fell to 0.83, paraphrase gained 0.02).
    // Paraphrase handling lives at the VERDICT level, where it works: only
    // 1.1% of paraphrases read as Novel via the familiar_min_score path.
    // Downstream consumers should branch on `verdict`, not threshold this
    // scalar across families.
    let familiarity = traces
        .first()
        .map(|t| {
            if total_potential > 0.0 {
                (t.score / total_potential).min(1.0)
            } else {
                0.0
            }
        })
        .unwrap_or(0.0);

    let (verdict, odds_of_old) = match traces.first() {
        None => (Verdict::Novel, 0.0),
        Some(best) => {
            let runner_up = traces.get(1).map(|t| t.score).unwrap_or(0.0);
            let clear_lead = best.score >= runner_up * config.recognize_margin;
            if best.coverage >= config.recognize_coverage
                && best.score >= config.recognize_min_score
                && clear_lead
            {
                (
                    Verdict::Recognized {
                        memory_id: best.memory_id.clone(),
                    },
                    best.score,
                )
            } else if familiarity >= config.familiar_floor
                || best.score >= config.familiar_min_score
            {
                (Verdict::Familiar, best.score)
            } else {
                (Verdict::Novel, best.score)
            }
        }
    };

    // Evidence: strongest rows across all traces, capped.
    let mut evidence: Vec<Evidence> = acc.into_values().flat_map(|a| a.evidence).collect();
    evidence.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.feature.cmp(&b.feature))
    });
    evidence.truncate(config.max_evidence);
    traces.truncate(config.max_traces);

    RecognitionResult {
        verdict,
        familiarity,
        odds_of_old,
        novelty: 1.0 - familiarity,
        traces,
        evidence,
        stimulus_peaks: prints.peaks.len(),
        stimulus_pairs: prints.pair_hashes.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RecognitionConfig;

    fn fm(hash: u64, id: &str, label: &str, df: usize) -> FeatureMatch {
        FeatureMatch {
            hash,
            memory_id: id.into(),
            label: label.into(),
            doc_frequency: df,
        }
    }

    fn prints_with(n_pairs: usize) -> StimulusPrints {
        let cfg = RecognitionConfig::default();
        let mut content = String::new();
        for i in 0..(n_pairs + 2) {
            content.push_str(&format!("uniqueword{i} "));
        }
        let mut p = crate::fingerprint_stimulus(&content, &cfg);
        p.pair_hashes.truncate(n_pairs);
        p.gram_hashes.clear();
        p
    }

    #[test]
    fn rare_matches_outweigh_common_ones() {
        let prints = prints_with(10);
        // Trace A: 2 rare features. Trace B: 3 very common features.
        let pair_matches = vec![
            fm(1, "a", "pair: rare1", 1),
            fm(2, "a", "pair: rare2", 1),
            fm(3, "b", "pair: common1", 90),
            fm(4, "b", "pair: common2", 90),
            fm(5, "b", "pair: common3", 90),
        ];
        let r = score_candidates(&prints, &pair_matches, &[], 100, &ScoreConfig::default());
        assert_eq!(r.traces[0].memory_id, "a", "rarity must beat raw count");
    }

    #[test]
    fn margin_rule_blocks_ambiguous_recognition() {
        let prints = prints_with(10);
        // Two traces with nearly identical strong evidence — must NOT
        // produce Recognized (ACR anti-flapping).
        let pair_matches: Vec<FeatureMatch> = (0..8)
            .flat_map(|i| {
                vec![
                    fm(i, "a", &format!("pair: f{i}"), 1),
                    fm(i + 100, "b", &format!("pair: g{i}"), 1),
                ]
            })
            .collect();
        let r = score_candidates(&prints, &pair_matches, &[], 100, &ScoreConfig::default());
        assert!(
            !matches!(r.verdict, Verdict::Recognized { .. }),
            "ambiguous dual-match must not lock: {:?}",
            r.verdict
        );
        assert_eq!(r.verdict, Verdict::Familiar);
    }

    #[test]
    fn no_matches_is_novel_with_full_novelty() {
        let prints = prints_with(10);
        let r = score_candidates(&prints, &[], &[], 100, &ScoreConfig::default());
        assert_eq!(r.verdict, Verdict::Novel);
        assert_eq!(r.familiarity, 0.0);
        assert_eq!(r.novelty, 1.0);
    }

    #[test]
    fn gram_hits_weigh_double() {
        let prints = prints_with(10);
        let pair = vec![fm(1, "a", "pair: x", 1)];
        let gram = vec![fm(2, "b", "run: 'x y z'", 1)];
        let r = score_candidates(&prints, &pair, &gram, 100, &ScoreConfig::default());
        let a = r.traces.iter().find(|t| t.memory_id == "a").unwrap();
        let b = r.traces.iter().find(|t| t.memory_id == "b").unwrap();
        assert!(b.score > a.score * 1.9, "gram evidence must weigh ~2x");
    }
}
