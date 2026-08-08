//! Outcome-ledger statistics (G3).
//!
//! Spectral has the only delivered-vs-used ledger in the field. It is scored
//! with a flat additive nudge — `signal_score = MIN(signal_score + strength,
//! 1.0)` (`sqlite_store.rs::reinforce_memory`) — which has three defects that
//! compound:
//!
//! 1. **No denominator.** A memory used once out of one delivery gets the same
//!    increment as one used ninety times out of a hundred. The estimator below
//!    uses a **Wilson lower bound**, which is small when evidence is thin and
//!    approaches the raw rate only as deliveries accumulate.
//! 2. **No exposure correction.** A memory delivered at rank 1 is more likely
//!    to be used *because it was seen first*. Reinforcing on the raw rate
//!    learns "rank-1 items are good", which is circular — the ranker taught
//!    itself its own output.
//! 3. **Unbounded compounding.** Reinforced → ranked higher → delivered higher
//!    → more likely used → reinforced. Rich-get-richer with no saturation.
//!
//! These are textbook defects with textbook repairs, and all three repairs are
//! deterministic, offline and free — no model, no network.
//!
//! **The position-bias repair is implemented but NOT yet usable on real data.**
//! Estimating the exposure curve requires adjudicated deliveries at each rank;
//! the live ledger has 4 per rank. See `g3-ledger-stats-2026-08-08.md` for the
//! measured data requirement.

/// Wilson score interval, lower bound, at ~95% (z = 1.96).
///
/// The point of using the *lower* bound rather than the raw rate is that it
/// encodes sample size directly: 1/1 scores 0.207, 90/100 scores 0.825. The
/// flat nudge treats those identically, which is defect (1).
///
/// `deliveries == 0` yields 0.0 — no evidence is not weak evidence of quality,
/// it is no evidence.
pub fn wilson_lower_bound(used: u64, deliveries: u64) -> f64 {
    wilson_lower_bound_z(used, deliveries, 1.96)
}

/// Wilson lower bound with an explicit z. Exposed for tests and for callers
/// that want a different confidence level.
pub fn wilson_lower_bound_z(used: u64, deliveries: u64, z: f64) -> f64 {
    if deliveries == 0 || used > deliveries {
        return 0.0;
    }
    let n = deliveries as f64;
    let p = used as f64 / n;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = p + z2 / (2.0 * n);
    let margin = z * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt();
    ((centre - margin) / denom).clamp(0.0, 1.0)
}

/// Saturating volume term: `log10(1 + used)`, normalised so 0 uses → 0.0.
///
/// Caps the rich-get-richer loop. Going from 1 use to 10 is a large change in
/// confidence; going from 100 to 1000 is not, and a linear term treats them as
/// equal. Unbounded above by design — the caller weights it.
pub fn saturating_volume(used: u64) -> f64 {
    ((1 + used) as f64).log10()
}

/// Empirical use-rate at each delivered rank — the exposure curve.
///
/// `adjudicated[r] = (used_at_r, delivered_at_r)`. Ranks with no adjudicated
/// deliveries yield `None` rather than 0.0: an unobserved rank is undefined,
/// not a rank where nothing was ever used. Conflating those is the same class
/// of error as R15's diluted denominator.
pub fn exposure_curve(adjudicated: &[(u64, u64)]) -> Vec<Option<f64>> {
    adjudicated
        .iter()
        .map(|&(used, delivered)| {
            if delivered == 0 {
                None
            } else {
                Some(used as f64 / delivered as f64)
            }
        })
        .collect()
}

/// Minimum adjudicated deliveries **per rank** needed to distinguish two
/// use-rates at 95% confidence with 80% power (two-proportion normal
/// approximation).
///
/// This exists so the answer to "can we do G3 yet?" is a number rather than an
/// intuition. Returns `None` if the rates are equal.
pub fn deliveries_needed_per_rank(p_high: f64, p_low: f64) -> Option<u64> {
    let d = (p_high - p_low).abs();
    if d <= f64::EPSILON {
        return None;
    }
    // z_{α/2} = 1.96, z_β = 0.84 for 80% power.
    let (za, zb) = (1.96_f64, 0.84_f64);
    let pbar = (p_high + p_low) / 2.0;
    let a = za * (2.0 * pbar * (1.0 - pbar)).sqrt();
    let b = zb * (p_high * (1.0 - p_high) + p_low * (1.0 - p_low)).sqrt();
    Some((((a + b) / d).powi(2)).ceil() as u64)
}

/// Position-corrected quality estimate for one memory.
///
/// Divides the observed use-rate by the exposure the memory actually received,
/// so a memory used 3 times from rank 1 is not credited above one used twice
/// from rank 30. `rank_deliveries[r]` is how many times this memory was
/// delivered at rank `r`; `curve` is the corpus-wide exposure curve.
///
/// Returns `None` when the exposure curve has no estimate for the ranks this
/// memory was delivered at — the honest answer when the correction cannot be
/// computed, rather than silently falling back to the uncorrected rate.
pub fn position_corrected_rate(
    used: u64,
    rank_deliveries: &[u64],
    curve: &[Option<f64>],
) -> Option<f64> {
    let mut expected = 0.0;
    let mut covered = 0u64;
    for (r, &n) in rank_deliveries.iter().enumerate() {
        if n == 0 {
            continue;
        }
        // `?` propagates the refusal: a rank with no exposure estimate means
        // the correction cannot be computed for this memory at all, and
        // returning the uncorrected rate instead would be worse than nothing.
        let rate = curve.get(r).copied().flatten()?;
        expected += rate * n as f64;
        covered += n;
    }
    if covered == 0 || expected <= 0.0 {
        return None;
    }
    // >1.0 means "used more than its rank alone predicts" — the signal we
    // actually want to reinforce on.
    Some(used as f64 / expected)
}

/// Combined ledger score: confidence-bounded, exposure-corrected, saturated.
///
/// `lift` is [`position_corrected_rate`] when computable. When it is not, the
/// score degrades to the Wilson bound alone rather than pretending to a
/// correction it cannot make.
pub fn ledger_score(used: u64, deliveries: u64, lift: Option<f64>, volume_weight: f64) -> f64 {
    let base = wilson_lower_bound(used, deliveries);
    let corrected = match lift {
        Some(l) => base * l,
        None => base,
    };
    corrected * (1.0 + volume_weight * saturating_volume(used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_punishes_thin_evidence() {
        let one_of_one = wilson_lower_bound(1, 1);
        let ninety_of_hundred = wilson_lower_bound(90, 100);
        assert!(
            one_of_one < ninety_of_hundred,
            "1/1 ({one_of_one}) must not outrank 90/100 ({ninety_of_hundred}) — \
             this is the exact defect the flat nudge has"
        );
        // Sanity against published values.
        assert!((one_of_one - 0.2065).abs() < 0.01, "got {one_of_one}");
        assert!(
            (ninety_of_hundred - 0.8247).abs() < 0.01,
            "got {ninety_of_hundred}"
        );
    }

    #[test]
    fn wilson_rises_toward_the_raw_rate_as_evidence_accumulates() {
        let seq = [
            wilson_lower_bound(1, 1),
            wilson_lower_bound(10, 10),
            wilson_lower_bound(100, 100),
            wilson_lower_bound(1000, 1000),
        ];
        for w in seq.windows(2) {
            assert!(w[0] < w[1], "must be monotone in sample size: {seq:?}");
        }
        assert!(*seq.last().unwrap() < 1.0, "never reaches certainty");
    }

    #[test]
    fn no_deliveries_is_no_evidence_not_weak_evidence() {
        assert_eq!(wilson_lower_bound(0, 0), 0.0);
        assert_eq!(
            wilson_lower_bound(5, 0),
            0.0,
            "malformed input must not score"
        );
    }

    #[test]
    fn volume_saturates() {
        let first = saturating_volume(10) - saturating_volume(1);
        let later = saturating_volume(1000) - saturating_volume(991);
        assert!(
            first > later * 5.0,
            "early uses must matter far more than late ones: {first} vs {later}"
        );
    }

    #[test]
    fn unobserved_ranks_are_undefined_not_zero() {
        let curve = exposure_curve(&[(1, 4), (0, 0), (2, 8)]);
        assert_eq!(curve[0], Some(0.25));
        assert_eq!(curve[1], None, "a rank never delivered at is UNDEFINED");
        assert_eq!(curve[2], Some(0.25));
    }

    #[test]
    fn position_correction_credits_the_deep_memory() {
        // Corpus: rank 0 is used 40% of the time, rank 9 only 5%.
        let mut curve = vec![None; 10];
        curve[0] = Some(0.40);
        curve[9] = Some(0.05);

        // A: used 4 times from 10 deliveries at rank 0 — exactly what rank
        // alone predicts. B: used 2 times from 10 at rank 9 — four times what
        // its exposure predicts.
        let mut a_ranks = vec![0u64; 10];
        a_ranks[0] = 10;
        let mut b_ranks = vec![0u64; 10];
        b_ranks[9] = 10;

        let a = position_corrected_rate(4, &a_ranks, &curve).unwrap();
        let b = position_corrected_rate(2, &b_ranks, &curve).unwrap();
        assert!(
            (a - 1.0).abs() < 1e-9,
            "A performs exactly at expectation, got {a}"
        );
        assert!(b > a, "B ({b}) beat its exposure and must outrank A ({a})");
        assert!((b - 4.0).abs() < 1e-9, "got {b}");
    }

    #[test]
    fn correction_refuses_rather_than_guesses_when_the_curve_is_missing() {
        let curve = vec![None, None, None];
        let ranks = vec![0u64, 5, 0];
        assert_eq!(
            position_corrected_rate(3, &ranks, &curve),
            None,
            "must not silently fall back to the uncorrected rate"
        );
    }

    #[test]
    fn ledger_score_degrades_gracefully_without_a_curve() {
        let with = ledger_score(9, 10, Some(1.0), 0.5);
        let without = ledger_score(9, 10, None, 0.5);
        assert!((with - without).abs() < 1e-9, "lift 1.0 == no lift");
        assert!(without > 0.0);
    }

    #[test]
    fn ledger_score_ranks_thin_evidence_below_thick() {
        let thin = ledger_score(1, 1, None, 0.5);
        let thick = ledger_score(90, 100, None, 0.5);
        assert!(thin < thick, "thin {thin} must rank below thick {thick}");
    }

    #[test]
    fn sample_size_requirement_is_a_number_not_an_intuition() {
        // Distinguishing a 25% rank-1 use-rate from a 10% deep-rank one.
        let n = deliveries_needed_per_rank(0.25, 0.10).unwrap();
        assert!(
            (80..=200).contains(&n),
            "expected ~100 adjudicated deliveries per rank, got {n}"
        );
        assert_eq!(
            deliveries_needed_per_rank(0.2, 0.2),
            None,
            "equal rates need no n"
        );
        // Smaller effects cost more data.
        let big = deliveries_needed_per_rank(0.25, 0.10).unwrap();
        let small = deliveries_needed_per_rank(0.25, 0.20).unwrap();
        assert!(small > big, "a subtler difference must demand more data");
    }
}
