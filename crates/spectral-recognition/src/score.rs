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

/// A MinHash lexical match: a candidate memory and the shingle-set
/// **containment** of the stimulus in it (fraction of the probe's shingles the
/// candidate contains — high for a partial/degraded re-encounter).
#[derive(Debug, Clone)]
pub struct MinHashMatch {
    pub memory_id: String,
    pub similarity: f64,
}

/// Thresholds and weights for verdict formation.
#[derive(Debug, Clone)]
pub struct ScoreConfig {
    /// Minimum coverage (fraction of stimulus fingerprints matched by one
    /// trace) for a `Recognized` verdict.
    pub recognize_coverage: f64,
    /// Minimum rarity-weighted score for a `Recognized` verdict (guards
    /// against tiny stimuli where coverage is trivially high).
    pub recognize_min_score: f64,
    /// Minimum normalized familiarity for an identity-bearing `Recognized`
    /// verdict. Absolute evidence alone can be high for a same-template but
    /// different event; this gate reserves identity for probes whose matched
    /// evidence covers enough of the stimulus. Lower-confidence echoes remain
    /// `Familiar` rather than being discarded as novel.
    pub recognize_min_familiarity: f64,
    /// Lead margin: best trace must exceed runner-up's score by this factor
    /// to be `Recognized` (ACR's θ+δ margin rule; prevents flapping between
    /// two similar traces).
    pub recognize_margin: f64,
    /// Familiarity floor for a `Familiar` verdict (coverage channel — the
    /// normalized, scale-independent arm).
    pub familiar_floor: f64,
    /// Similarity floor for a `Familiar` verdict via the MinHash channel.
    /// Decoupled from `familiar_floor`: raw token overlap between
    /// same-domain short texts trivially reaches ~0.10 by chance, so the
    /// similarity arm needs a higher bar than normalized coverage
    /// (measured: 99.3% false-familiar on the public R1 negatives under a
    /// shared 0.10 floor — see recognition-verdict-calibration-prereg).
    pub familiar_min_similarity: f64,
    /// Alternative Familiar path: best-trace rarity-weighted score at or
    /// above this triggers Familiar even at low coverage (REM: a couple of
    /// very rare matched anchors are strong evidence despite covering
    /// little of the stimulus).
    pub familiar_min_score: f64,
    /// Minimum independent matched features (pair + gram hits) for the
    /// Familiar-by-score path. Rarity weights grow as ln(enrolled/df), so at
    /// large enrollment a SINGLE chance collision clears any constant score
    /// threshold; requiring two independent features makes the path
    /// scale-robust without touching the scalar.
    pub familiar_min_features: usize,
    /// Winnowed-gram hits weigh this multiple of an equally-rare pair hit
    /// (verbatim runs are stronger identity evidence than co-occurrence).
    pub gram_weight: f64,
    /// Maximum evidence rows returned (strongest first).
    pub max_evidence: usize,
    /// Maximum candidate traces returned.
    pub max_traces: usize,
    /// R42: when the plain lead margin fails, decide it again on **exclusive**
    /// evidence — the features matched by one of the top two candidates and
    /// not the other.
    ///
    /// Two memories that share a template (`Started working in project X`,
    /// `Navigated to <url>`) accumulate nearly all of their score from
    /// features they both match, so their totals tie and the margin rule
    /// cannot see the URL or id that actually distinguishes them. Measured on
    /// the real brain: 100% of exact-re-encounter misses were lead-margin
    /// failures against a near-duplicate, and 44% of those had distinguishing
    /// evidence available.
    ///
    /// The promotion reuses the existing bars — exclusive evidence must clear
    /// `recognize_margin` relative to the rival's exclusive evidence AND
    /// `recognize_min_score` absolutely — so it introduces no new constant.
    /// Byte-identical candidates have zero exclusive evidence and are never
    /// promoted, which is the correct outcome: nothing in the content can
    /// choose between them.
    pub discriminative_margin: bool,
}

impl Default for ScoreConfig {
    fn default() -> Self {
        Self {
            recognize_coverage: 0.35,
            recognize_min_score: 3.0,
            recognize_min_familiarity: 0.60,
            recognize_margin: 1.5,
            familiar_floor: 0.10,
            familiar_min_similarity: 0.20,
            familiar_min_score: 2.5,
            familiar_min_features: 2,
            gram_weight: 2.0,
            max_evidence: 12,
            max_traces: 5,
            discriminative_margin: false,
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

/// Rarity-weighted evidence each of two candidates matched that the other did
/// NOT (R42). Keyed by `(hash, is_gram)` so the same hash appearing as both a
/// pair and a gram feature stays distinct; a feature matched more than once by
/// the same memory counts once, at its weight.
///
/// The MinHash channel is deliberately excluded: containment is a
/// whole-document similarity, not a feature, so "exclusive" is undefined for
/// it — and it is precisely the channel that near-duplicates share.
fn exclusive_evidence(
    pair_matches: &[FeatureMatch],
    gram_matches: &[FeatureMatch],
    enrolled: usize,
    config: &ScoreConfig,
    best_id: &str,
    runner_id: &str,
) -> (f64, f64) {
    let mut best_f: HashMap<(u64, bool), f64> = HashMap::new();
    let mut runner_f: HashMap<(u64, bool), f64> = HashMap::new();
    for (matches, is_gram) in [(pair_matches, false), (gram_matches, true)] {
        for m in matches {
            let target = if m.memory_id == best_id {
                &mut best_f
            } else if m.memory_id == runner_id {
                &mut runner_f
            } else {
                continue;
            };
            let base = rarity(enrolled, m.doc_frequency);
            let w = if is_gram {
                base * config.gram_weight
            } else {
                base
            };
            let e = target.entry((m.hash, is_gram)).or_insert(0.0);
            *e = e.max(w);
        }
    }
    let excl = |a: &HashMap<(u64, bool), f64>, b: &HashMap<(u64, bool), f64>| -> f64 {
        a.iter()
            .filter(|(k, _)| !b.contains_key(*k))
            .map(|(_, w)| *w)
            .sum()
    };
    (excl(&best_f, &runner_f), excl(&runner_f, &best_f))
}

/// Score candidates and form a verdict.
///
/// `minhash_matches` are the shingle-containment channel (candidate memory +
/// containment of the stimulus in it); `minhash_weight` scales a full
/// (containment 1.0) match relative to a maximally-rare pair hit, and
/// `min_similarity` gates evidence. Pass an empty slice / `minhash_weight =
/// 0.0` to run the legacy pair+gram-only engine.
#[allow(clippy::too_many_arguments)]
pub fn score_candidates(
    prints: &StimulusPrints,
    pair_matches: &[FeatureMatch],
    gram_matches: &[FeatureMatch],
    minhash_matches: &[MinHashMatch],
    enrolled: usize,
    config: &ScoreConfig,
    minhash_weight: f64,
    min_similarity: f64,
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

    // MinHash channel: a strong, normalized lexical-overlap signal. A match's
    // contribution scales with its estimated Jaccard and the maximum rarity
    // unit, so a near-identical re-encounter contributes as much as several
    // rare pair hits, while a topical near-miss (low Jaccard) contributes
    // little. `best_similarity` also lifts the familiarity scalar below.
    let rarity_unit = rarity(enrolled, 1);
    let mut best_similarity = 0.0f64;
    if minhash_weight > 0.0 {
        for mm in minhash_matches {
            if mm.similarity < min_similarity {
                continue;
            }
            best_similarity = best_similarity.max(mm.similarity);
            let w = mm.similarity * minhash_weight * rarity_unit;
            let a = acc.entry(mm.memory_id.clone()).or_insert(Accum {
                score: 0.0,
                pair_hits: 0,
                gram_hits: 0,
                evidence: Vec::new(),
            });
            a.score += w;
            a.evidence.push(Evidence {
                feature: format!("minhash: containment {:.2}", mm.similarity),
                memory_id: mm.memory_id.clone(),
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
    let coverage_familiarity = traces
        .first()
        .map(|t| {
            if total_potential > 0.0 {
                (t.score / total_potential).min(1.0)
            } else {
                0.0
            }
        })
        .unwrap_or(0.0);
    // Fuse the coverage-based scalar with the MinHash lexical scalar by taking
    // the stronger channel: a true re-encounter reads high on at least one
    // (geometric coverage OR token-set overlap), while a topical near-miss
    // reads low on both. MinHash is a normalized [0,1] similarity (not the
    // rejected absolute-evidence blend), and is the sharper lexical
    // discriminator (RECOGNITION_BASELINE: AUC ~0.998 vs peak-pair ~0.941).
    let familiarity = coverage_familiarity.max(best_similarity);

    let (verdict, odds_of_old) = match traces.first() {
        None => (Verdict::Novel, 0.0),
        Some(best) => {
            let runner_up = traces.get(1).map(|t| t.score).unwrap_or(0.0);
            let clear_lead = best.score >= runner_up * config.recognize_margin
                || (config.discriminative_margin
                    && match traces.get(1) {
                        // No rival: the plain rule already decided.
                        None => false,
                        Some(rival) => {
                            let (excl_best, excl_rival) = exclusive_evidence(
                                pair_matches,
                                gram_matches,
                                enrolled,
                                config,
                                &best.memory_id,
                                &rival.memory_id,
                            );
                            excl_best >= config.recognize_min_score
                                && excl_best >= excl_rival * config.recognize_margin
                        }
                    });
            if best.coverage >= config.recognize_coverage
                && best.score >= config.recognize_min_score
                && familiarity >= config.recognize_min_familiarity
                && clear_lead
            {
                (
                    Verdict::Recognized {
                        memory_id: best.memory_id.clone(),
                    },
                    best.score,
                )
            } else {
                // Familiar requires scale-robust evidence (see prereg
                // recognition-verdict-calibration-2026-07-29):
                // - coverage channel: normalized, scale-independent — floor
                //   unchanged;
                // - similarity channel: raw token overlap needs a higher bar
                //   than coverage (0.10 is chance-level between same-domain
                //   short texts);
                // - by-score channel: rarity weights grow with ln(enrolled),
                //   so the absolute threshold additionally requires at least
                //   `familiar_min_features` independent matched features — a
                //   single chance collision no longer suffices at scale.
                let features = best.pair_hits + best.gram_hits;
                let by_coverage = coverage_familiarity >= config.familiar_floor;
                let by_similarity = best_similarity >= config.familiar_min_similarity;
                let by_score = best.score >= config.familiar_min_score
                    && features >= config.familiar_min_features;
                if by_coverage || by_similarity || by_score {
                    (Verdict::Familiar, best.score)
                } else {
                    (Verdict::Novel, best.score)
                }
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
            // `acc` is a HashMap, so its iteration order is randomly seeded.
            // Two memories sharing a pair hash yield Evidence rows with equal
            // `weight` (doc frequency is per-hash) AND equal `feature`, so
            // without a total order the stable sort preserves that random
            // order and the `truncate` below drops a nondeterministic subset
            // of the audit trail. `traces` above already tiebreaks on
            // memory_id for the same reason.
            .then_with(|| a.memory_id.cmp(&b.memory_id))
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
        let r = score_candidates(
            &prints,
            &pair_matches,
            &[],
            &[],
            100,
            &ScoreConfig::default(),
            0.0,
            0.0,
        );
        assert_eq!(r.traces[0].memory_id, "a", "rarity must beat raw count");
    }

    /// R42: two candidates sharing most of their evidence tie on TOTALS, so
    /// the plain margin refuses identity — but one of them has strictly more
    /// evidence the other lacks. With `discriminative_margin` on, that
    /// exclusive evidence decides, and it must clear the SAME two bars
    /// (`recognize_margin` relative, `recognize_min_score` absolute).
    #[test]
    fn discriminative_margin_decides_on_exclusive_evidence() {
        let prints = prints_with(24);
        let mut matches = Vec::new();
        // 20 shared features: both candidates match them, so they cancel.
        for h in 0..20u64 {
            matches.push(fm(h, "a", "pair: shared", 3));
            matches.push(fm(h, "b", "pair: shared", 3));
        }
        // Exclusive: two rare features for `a`, one for `b`.
        matches.push(fm(100, "a", "pair: only-a-1", 1));
        matches.push(fm(101, "a", "pair: only-a-2", 1));
        matches.push(fm(102, "b", "pair: only-b-1", 1));

        let baseline = score_candidates(
            &prints,
            &matches,
            &[],
            &[],
            1000,
            &ScoreConfig::default(),
            0.0,
            0.0,
        );
        assert_eq!(baseline.traces[0].memory_id, "a");
        assert!(
            matches!(baseline.verdict, Verdict::Familiar),
            "precondition: shared evidence must sink the totals below the {}x margin, got {:?}",
            ScoreConfig::default().recognize_margin,
            baseline.verdict
        );

        let cfg = ScoreConfig {
            discriminative_margin: true,
            ..ScoreConfig::default()
        };
        let promoted = score_candidates(&prints, &matches, &[], &[], 1000, &cfg, 0.0, 0.0);
        assert_eq!(
            promoted.verdict,
            Verdict::Recognized {
                memory_id: "a".to_string()
            },
            "exclusive evidence 2 rare vs 1 rare should clear both bars"
        );
    }

    /// A gram carries `gram_weight` times a pair of equal rarity, and that
    /// multiplication decides promotions near the bar.
    ///
    /// Chosen so the three arithmetic readings disagree: best's only exclusive
    /// feature is a GRAM at df 50 (rarity 2.997), rival's is a PAIR at df 30
    /// (rarity 3.508, so the bar is 5.261). Multiplying gives 5.993 and
    /// promotes; adding gives 4.997 and does not; dividing gives 1.498 and does
    /// not. A test built on round numbers would have let two of the three pass.
    #[test]
    fn a_gram_is_weighted_not_merely_counted_in_exclusive_evidence() {
        let prints = prints_with(24);
        let mut pairs = Vec::new();
        let mut grams = Vec::new();
        for h in 0..20u64 {
            pairs.push(fm(h, "a", "pair: shared", 3));
            pairs.push(fm(h, "b", "pair: shared", 3));
        }
        grams.push(fm(100, "a", "run: only-a", 50));
        pairs.push(fm(101, "b", "pair: only-b", 30));

        let cfg = ScoreConfig {
            discriminative_margin: true,
            ..ScoreConfig::default()
        };
        let r = score_candidates(&prints, &pairs, &grams, &[], 1000, &cfg, 0.0, 0.0);
        assert_eq!(
            r.verdict,
            Verdict::Recognized {
                memory_id: "a".to_string()
            },
            "the exclusive gram must be weighted by gram_weight, not counted flat"
        );
    }

    /// The exclusive lead is a MULTIPLE of the rival's, not a margin added to
    /// it — and a candidate that leads by less than the factor stays Familiar.
    ///
    /// Rival's exclusive pair is df 18 (rarity 4.018), best's is df 3 (5.810).
    /// The bar is 4.018 x 1.5 = 6.028, so best falls short and must NOT be
    /// promoted. Reading the operator as `+` gives a bar of 5.518 and reading
    /// it as `/` gives 2.679 — both would promote. This is the case that says
    /// the rule is a ratio.
    #[test]
    fn an_exclusive_lead_below_the_margin_factor_does_not_promote() {
        let prints = prints_with(24);
        let mut matches = Vec::new();
        for h in 0..20u64 {
            matches.push(fm(h, "a", "pair: shared", 3));
            matches.push(fm(h, "b", "pair: shared", 3));
        }
        matches.push(fm(100, "a", "pair: only-a", 3));
        matches.push(fm(101, "b", "pair: only-b", 18));

        let cfg = ScoreConfig {
            discriminative_margin: true,
            ..ScoreConfig::default()
        };
        let r = score_candidates(&prints, &matches, &[], &[], 1000, &cfg, 0.0, 0.0);
        assert_eq!(
            r.traces[0].memory_id, "a",
            "precondition: a still leads on totals"
        );
        assert!(
            matches!(r.verdict, Verdict::Familiar),
            "a lead of 1.45x must not clear a 1.5x bar, got {:?}",
            r.verdict
        );
    }

    /// R42 safety property: candidates whose evidence is IDENTICAL have zero
    /// exclusive evidence, so the rule must not fire — it can only break a tie
    /// the content actually distinguishes, never invent one.
    #[test]
    fn discriminative_margin_cannot_promote_identical_evidence() {
        let prints = prints_with(24);
        let mut matches = Vec::new();
        for h in 0..20u64 {
            matches.push(fm(h, "a", "pair: shared", 3));
            matches.push(fm(h, "b", "pair: shared", 3));
        }
        let cfg = ScoreConfig {
            discriminative_margin: true,
            ..ScoreConfig::default()
        };
        let r = score_candidates(&prints, &matches, &[], &[], 1000, &cfg, 0.0, 0.0);
        assert!(
            matches!(r.verdict, Verdict::Familiar),
            "identical evidence must stay Familiar, got {:?}",
            r.verdict
        );
    }

    /// A single exclusive feature must not be enough: the absolute bar
    /// (`recognize_min_score`) still applies to exclusive evidence, so a lone
    /// chance match cannot name an identity.
    #[test]
    fn discriminative_margin_respects_the_absolute_bar() {
        let prints = prints_with(24);
        let mut matches = Vec::new();
        for h in 0..20u64 {
            matches.push(fm(h, "a", "pair: shared", 3));
            matches.push(fm(h, "b", "pair: shared", 3));
        }
        // `a`'s only exclusive feature is COMMON (df 900): weight ~0.1, far
        // below recognize_min_score. `b` has none at all, so the relative bar
        // is trivially met — only the absolute bar can refuse this.
        matches.push(fm(100, "a", "pair: only-a-but-common", 900));
        let cfg = ScoreConfig {
            discriminative_margin: true,
            ..ScoreConfig::default()
        };
        let r = score_candidates(&prints, &matches, &[], &[], 1000, &cfg, 0.0, 0.0);
        assert!(
            matches!(r.verdict, Verdict::Familiar),
            "one common exclusive feature must not name an identity, got {:?}",
            r.verdict
        );
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
        let r = score_candidates(
            &prints,
            &pair_matches,
            &[],
            &[],
            100,
            &ScoreConfig::default(),
            0.0,
            0.0,
        );
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
        let r = score_candidates(
            &prints,
            &[],
            &[],
            &[],
            100,
            &ScoreConfig::default(),
            0.0,
            0.0,
        );
        assert_eq!(r.verdict, Verdict::Novel);
        assert_eq!(r.familiarity, 0.0);
        assert_eq!(r.novelty, 1.0);
    }

    #[test]
    fn gram_hits_weigh_double() {
        let prints = prints_with(10);
        let pair = vec![fm(1, "a", "pair: x", 1)];
        let gram = vec![fm(2, "b", "run: 'x y z'", 1)];
        let r = score_candidates(
            &prints,
            &pair,
            &gram,
            &[],
            100,
            &ScoreConfig::default(),
            0.0,
            0.0,
        );
        let a = r.traces.iter().find(|t| t.memory_id == "a").unwrap();
        let b = r.traces.iter().find(|t| t.memory_id == "b").unwrap();
        assert!(b.score > a.score * 1.9, "gram evidence must weigh ~2x");
    }

    #[test]
    fn absolute_evidence_cannot_claim_identity_at_low_familiarity() {
        let prints = prints_with(10);
        // Four rare matches clear the legacy coverage, score, and margin
        // gates, but cover too little of the probe to establish identity.
        let pair_matches: Vec<FeatureMatch> = (0..4)
            .map(|i| fm(i, "same-template", &format!("pair: f{i}"), 1))
            .collect();
        let r = score_candidates(
            &prints,
            &pair_matches,
            &[],
            &[],
            100,
            &ScoreConfig::default(),
            0.0,
            0.0,
        );
        assert!(r.familiarity < ScoreConfig::default().recognize_min_familiarity);
        assert_eq!(r.verdict, Verdict::Familiar);
    }
}
