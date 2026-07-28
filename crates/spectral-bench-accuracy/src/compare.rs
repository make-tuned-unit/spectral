//! Paired A/B comparison of two eval reports.
//!
//! Codifies the paired-analysis methodology used for every A/B verdict:
//! clean-intersection join by question_id (transport/auth/judge failures
//! excluded from both arms), per-arm accuracy with Wilson 95% CIs,
//! discordant-pair breakdown, and an exact two-sided McNemar test.
//!
//! This is an analysis tool, not a gate: it always exits 0.

use crate::report::{load_report, EvalReport, OutcomeClass, QuestionResult};
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;

/// Result of joining two report result sets on question_id.
#[derive(Debug, Clone)]
pub struct PairedAnalysis {
    /// Question ids present in exactly one report (not joinable).
    pub unmatched_a: Vec<String>,
    pub unmatched_b: Vec<String>,
    /// Joined ids dropped because outcome_class != Ok in either arm.
    pub dropped: Vec<String>,
    /// Clean intersection size (denominator for everything below).
    pub n: usize,
    pub a_correct: usize,
    pub b_correct: usize,
    pub both_correct: usize,
    pub both_wrong: usize,
    /// Correct in A only (B regressed these).
    pub a_only: Vec<String>,
    /// Correct in B only (B recovered these).
    pub b_only: Vec<String>,
}

impl PairedAnalysis {
    pub fn discordant(&self) -> usize {
        self.a_only.len() + self.b_only.len()
    }

    /// Exact two-sided McNemar p-value on the discordant pairs.
    pub fn mcnemar_p(&self) -> f64 {
        mcnemar_exact_p(self.discordant(), self.a_only.len().min(self.b_only.len()))
    }
}

/// Join two result sets by question_id, keeping only the clean intersection:
/// questions present in both arms with outcome_class == Ok in both.
pub fn paired_analysis(a: &[QuestionResult], b: &[QuestionResult]) -> PairedAnalysis {
    // BTreeMap for deterministic id ordering in all printed lists.
    let a_by_id: BTreeMap<&str, &QuestionResult> =
        a.iter().map(|r| (r.question_id.as_str(), r)).collect();
    let b_by_id: BTreeMap<&str, &QuestionResult> =
        b.iter().map(|r| (r.question_id.as_str(), r)).collect();

    let unmatched_a: Vec<String> = a_by_id
        .keys()
        .filter(|id| !b_by_id.contains_key(*id))
        .map(|id| id.to_string())
        .collect();
    let unmatched_b: Vec<String> = b_by_id
        .keys()
        .filter(|id| !a_by_id.contains_key(*id))
        .map(|id| id.to_string())
        .collect();

    let mut dropped = Vec::new();
    let mut n = 0;
    let mut a_correct = 0;
    let mut b_correct = 0;
    let mut both_correct = 0;
    let mut both_wrong = 0;
    let mut a_only = Vec::new();
    let mut b_only = Vec::new();

    for (id, ra) in &a_by_id {
        let rb = match b_by_id.get(id) {
            Some(rb) => rb,
            None => continue,
        };
        if ra.outcome_class != OutcomeClass::Ok || rb.outcome_class != OutcomeClass::Ok {
            dropped.push(id.to_string());
            continue;
        }
        n += 1;
        match (ra.correct, rb.correct) {
            (true, true) => {
                a_correct += 1;
                b_correct += 1;
                both_correct += 1;
            }
            (true, false) => {
                a_correct += 1;
                a_only.push(id.to_string());
            }
            (false, true) => {
                b_correct += 1;
                b_only.push(id.to_string());
            }
            (false, false) => both_wrong += 1,
        }
    }

    PairedAnalysis {
        unmatched_a,
        unmatched_b,
        dropped,
        n,
        a_correct,
        b_correct,
        both_correct,
        both_wrong,
        a_only,
        b_only,
    }
}

/// Exact two-sided McNemar p-value: p = min(1, 2 * sum_{i=0..k} C(n,i) / 2^n)
/// where n = discordant count and k = min(|A-only|, |B-only|).
///
/// Binomial terms are accumulated iteratively in f64
/// (term_{i+1} = term_i * (n-i)/(i+1)), exact to f64 precision and safe
/// well beyond n = 200 (0.5^n only underflows past n ~ 1074).
pub fn mcnemar_exact_p(n: usize, k: usize) -> f64 {
    if n == 0 {
        return 1.0;
    }
    let mut term = 0.5_f64.powi(n as i32); // C(n,0) / 2^n
    let mut sum = term;
    for i in 0..k.min(n) {
        term *= (n - i) as f64 / (i + 1) as f64;
        sum += term;
    }
    (2.0 * sum).min(1.0)
}

/// Wilson 95% score interval for a binomial proportion.
pub fn wilson_ci_95(correct: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let z = 1.959_963_984_540_054_f64;
    let nf = n as f64;
    let p = correct as f64 / nf;
    let z2 = z * z;
    let denom = 1.0 + z2 / nf;
    let center = (p + z2 / (2.0 * nf)) / denom;
    let half = z * (p * (1.0 - p) / nf + z2 / (4.0 * nf * nf)).sqrt() / denom;
    ((center - half).max(0.0), (center + half).min(1.0))
}

fn fingerprint_warnings(a: &EvalReport, b: &EvalReport) -> Vec<String> {
    let mut warnings = Vec::new();
    // Empty string = unknown (older reports) — no warning either way.
    if !a.config_fingerprint.is_empty()
        && !b.config_fingerprint.is_empty()
        && a.config_fingerprint == b.config_fingerprint
    {
        warnings.push(format!(
            "WARNING: config_fingerprint is IDENTICAL in both reports ({}). \
             Comparing two runs of the same config is usually a mistake — \
             check you loaded the right files.",
            a.config_fingerprint
        ));
    }
    if !a.judge_rubric_fingerprint.is_empty()
        && !b.judge_rubric_fingerprint.is_empty()
        && a.judge_rubric_fingerprint != b.judge_rubric_fingerprint
    {
        warnings.push(format!(
            "WARNING: judge_rubric_fingerprint DIFFERS (A: {}, B: {}). \
             Grades are not comparable across rubrics — this delta is not \
             a clean A/B verdict.",
            a.judge_rubric_fingerprint, b.judge_rubric_fingerprint
        ));
    }
    warnings
}

fn format_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        "(none)".into()
    } else {
        ids.join(", ")
    }
}

/// Load two reports, run the paired analysis, and print the verdict.
/// Always returns Ok — this is an analysis tool, not a gate.
pub fn run_compare(a_path: &Path, b_path: &Path) -> Result<()> {
    let a = load_report(a_path)?;
    let b = load_report(b_path)?;

    println!("=== Paired A/B Comparison ===");
    println!("A: {} ({})", a_path.display(), a.actor_name);
    println!("B: {} ({})", b_path.display(), b.actor_name);
    println!();

    for w in fingerprint_warnings(&a, &b) {
        eprintln!("{w}");
        eprintln!();
    }

    let pa = paired_analysis(&a.results, &b.results);

    if !pa.unmatched_a.is_empty() || !pa.unmatched_b.is_empty() {
        println!(
            "Unmatched question_ids (excluded from join): {} only in A, {} only in B",
            pa.unmatched_a.len(),
            pa.unmatched_b.len()
        );
    }
    println!(
        "Dropped from clean intersection (outcome != ok in either arm): {}",
        pa.dropped.len()
    );
    if !pa.dropped.is_empty() {
        println!("  ids: {}", format_ids(&pa.dropped));
    }
    println!("Clean intersection: {} questions", pa.n);
    println!();

    let pct = |c: usize| {
        if pa.n == 0 {
            0.0
        } else {
            100.0 * c as f64 / pa.n as f64
        }
    };
    let (a_lo, a_hi) = wilson_ci_95(pa.a_correct, pa.n);
    let (b_lo, b_hi) = wilson_ci_95(pa.b_correct, pa.n);
    println!(
        "A: {}/{} ({:.1}%)  Wilson 95% CI [{:.1}%, {:.1}%]",
        pa.a_correct,
        pa.n,
        pct(pa.a_correct),
        a_lo * 100.0,
        a_hi * 100.0
    );
    println!(
        "B: {}/{} ({:.1}%)  Wilson 95% CI [{:.1}%, {:.1}%]",
        pa.b_correct,
        pa.n,
        pct(pa.b_correct),
        b_lo * 100.0,
        b_hi * 100.0
    );
    let delta = pa.b_correct as i64 - pa.a_correct as i64;
    println!(
        "Net delta (B - A): {:+} ({:+.1} pts)",
        delta,
        pct(pa.b_correct) - pct(pa.a_correct)
    );
    println!();

    println!("Both correct: {}", pa.both_correct);
    println!("Both wrong:   {}", pa.both_wrong);
    println!("A-only correct (B regressed): {}", pa.a_only.len());
    if !pa.a_only.is_empty() {
        println!("  ids: {}", format_ids(&pa.a_only));
    }
    println!("B-only correct (B recovered): {}", pa.b_only.len());
    if !pa.b_only.is_empty() {
        println!("  ids: {}", format_ids(&pa.b_only));
    }
    println!();

    let n_disc = pa.discordant();
    let k = pa.a_only.len().min(pa.b_only.len());
    let p = pa.mcnemar_p();
    println!("McNemar exact two-sided: n={n_disc} discordant, k={k}, p = {p:.4}");
    if n_disc == 0 {
        println!("  (no discordant pairs — arms are identical on the clean intersection)");
    } else if p < 0.05 {
        println!("  Significant at alpha = 0.05.");
    } else {
        println!("  Not significant at alpha = 0.05 — delta is consistent with noise.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Category;

    fn qr(id: &str, correct: bool, outcome: OutcomeClass) -> QuestionResult {
        QuestionResult {
            question_id: id.into(),
            category: Category::MultiSession,
            question: "Q?".into(),
            ground_truth: "gt".into(),
            predicted: "p".into(),
            correct,
            judge_reasoning: None,
            retrieved_memory_count: 0,
            retrieved_memory_keys: vec![],
            duration_ms: 0,
            cascade_telemetry: None,
            strategy_telemetry: None,
            retry_count: 0,
            outcome_class: outcome,
            actor_context: None,
            question_date: None,
            replayed_predicted: None,
            replayed_correct: None,
            replayed_judge_reasoning: None,
            efficiency: None,
        }
    }

    #[test]
    fn mcnemar_known_values() {
        // n=7, k=2: 2 * (1+7+21)/128 = 58/128 = 0.453125
        assert!((mcnemar_exact_p(7, 2) - 0.4531).abs() < 5e-5);
        // n=1, k=0: 2 * 1/2 = 1.0
        assert!((mcnemar_exact_p(1, 0) - 1.0).abs() < 1e-12);
        // n=2, k=0: 2 * 1/4 = 0.5
        assert!((mcnemar_exact_p(2, 0) - 0.5).abs() < 1e-12);
        // n=0: no discordant pairs, p = 1
        assert!((mcnemar_exact_p(0, 0) - 1.0).abs() < 1e-12);
        // Balanced discordance clamps at 1: n=4, k=2 → 2*(1+4+6)/16 = 1.375 → 1.0
        assert!((mcnemar_exact_p(4, 2) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn mcnemar_real_prior_result() {
        // A=18/78, B=20/78, 8 A-only / 10 B-only → n=18, k=8, p = 0.8145
        assert!((mcnemar_exact_p(18, 8) - 0.8145).abs() < 5e-5);
    }

    #[test]
    fn wilson_ci_sanity() {
        // 50/100: symmetric around 0.5, roughly [0.404, 0.596]
        let (lo, hi) = wilson_ci_95(50, 100);
        assert!((lo - 0.4038).abs() < 1e-3, "lo = {lo}");
        assert!((hi - 0.5962).abs() < 1e-3, "hi = {hi}");
        // 0/10: lower bound exactly 0, upper bound well above 0
        let (lo, hi) = wilson_ci_95(0, 10);
        assert!(lo.abs() < 1e-12);
        assert!(hi > 0.2 && hi < 0.4, "hi = {hi}");
        // 10/10: upper bound exactly 1
        let (lo, hi) = wilson_ci_95(10, 10);
        assert!((hi - 1.0).abs() < 1e-12);
        assert!(lo > 0.6 && lo < 0.8, "lo = {lo}");
        // n=0: degenerate, full interval
        assert_eq!(wilson_ci_95(0, 0), (0.0, 1.0));
    }

    #[test]
    fn clean_intersection_excludes_non_ok_and_unmatched() {
        let a = vec![
            qr("q1", true, OutcomeClass::Ok),
            qr("q2", false, OutcomeClass::Ok),
            qr("q3", true, OutcomeClass::TransportFailure), // non-Ok in A
            qr("q4", true, OutcomeClass::Ok),               // non-Ok in B
            qr("q5", true, OutcomeClass::Ok),               // only in A
        ];
        let b = vec![
            qr("q1", false, OutcomeClass::Ok),
            qr("q2", true, OutcomeClass::Ok),
            qr("q3", true, OutcomeClass::Ok),
            qr("q4", true, OutcomeClass::AuthFailure),
            qr("q6", true, OutcomeClass::Ok), // only in B
        ];
        let pa = paired_analysis(&a, &b);
        assert_eq!(pa.n, 2);
        assert_eq!(pa.dropped, vec!["q3".to_string(), "q4".to_string()]);
        assert_eq!(pa.unmatched_a, vec!["q5".to_string()]);
        assert_eq!(pa.unmatched_b, vec!["q6".to_string()]);
        assert_eq!(pa.a_only, vec!["q1".to_string()]);
        assert_eq!(pa.b_only, vec!["q2".to_string()]);
        assert_eq!(pa.a_correct, 1);
        assert_eq!(pa.b_correct, 1);
        assert_eq!(pa.both_correct, 0);
        assert_eq!(pa.both_wrong, 0);
        assert_eq!(pa.discordant(), 2);
    }

    #[test]
    fn paired_analysis_real_prior_shape() {
        // Reconstruct A=18/78, B=20/78 with 8 A-only / 10 B-only:
        // 10 both-correct, 8 A-only, 10 B-only, 50 both-wrong.
        let mut a = Vec::new();
        let mut b = Vec::new();
        for i in 0..78 {
            let id = format!("q{i:03}");
            let (ca, cb) = match i {
                0..=9 => (true, true),    // both correct
                10..=17 => (true, false), // A-only
                18..=27 => (false, true), // B-only
                _ => (false, false),      // both wrong
            };
            a.push(qr(&id, ca, OutcomeClass::Ok));
            b.push(qr(&id, cb, OutcomeClass::Ok));
        }
        let pa = paired_analysis(&a, &b);
        assert_eq!(pa.n, 78);
        assert_eq!(pa.a_correct, 18);
        assert_eq!(pa.b_correct, 20);
        assert_eq!(pa.both_correct, 10);
        assert_eq!(pa.both_wrong, 50);
        assert_eq!(pa.a_only.len(), 8);
        assert_eq!(pa.b_only.len(), 10);
        assert!((pa.mcnemar_p() - 0.8145).abs() < 5e-5);
    }
}
