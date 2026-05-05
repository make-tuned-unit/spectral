//! Signal score computation for memory ingestion.
//!
//! Scores memories by cognitive significance (0.0–1.0). Uses density-based
//! sigmoid contributions for smooth gradient output rather than fixed bumps.

/// Compute signal_score for a memory. Returns 0.0–1.0.
pub fn score_memory(content: &str, hall: &str) -> f64 {
    let hall_lower = hall.to_lowercase();
    let base: f64 = match hall_lower.as_str() {
        "fact" => 0.65,
        "discovery" => 0.62,
        "preference" => 0.58,
        "advice" => 0.55,
        "event" => 0.50,
        _ => 0.50,
    };

    let content_lower = content.to_lowercase();

    let decision_density = count_matches(
        &content_lower,
        &[
            "decided",
            "chose",
            "switched",
            "approved",
            "rejected",
            "picked",
            "going with",
        ],
    );
    let error_density = count_matches(
        &content_lower,
        &[
            "error", "bug", "failed", "broke", "crash", "broken", "issue",
        ],
    );
    let learning_density = count_matches(
        &content_lower,
        &[
            "learned",
            "realized",
            "breakthrough",
            "insight",
            "discovered",
            "found that",
        ],
    );
    let rule_density = count_matches(
        &content_lower,
        &["always", "never", "rule", "policy", "must", "require"],
    );
    let urgency_density = count_matches(
        &content_lower,
        &["deadline", "urgent", "critical", "blocker", "priority"],
    );

    let score = base
        + sigmoid_contribution(decision_density, 0.18)
        + sigmoid_contribution(error_density, 0.12)
        + sigmoid_contribution(learning_density, 0.12)
        + sigmoid_contribution(rule_density, 0.08)
        + sigmoid_contribution(urgency_density, 0.08);

    score.clamp(0.0, 1.0)
}

/// Sigmoid-shaped contribution: density of 1 match gives ~half the weight,
/// density of 2+ gives progressively more but always bounded by max_weight.
fn sigmoid_contribution(density: usize, max_weight: f64) -> f64 {
    if density == 0 {
        return 0.0;
    }
    let d = density as f64;
    // d / (d + 1.0) maps: 1→0.5, 2→0.67, 3→0.75, ...
    max_weight * (d / (d + 1.0))
}

fn count_matches(text: &str, words: &[&str]) -> usize {
    words.iter().filter(|w| text.contains(**w)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_scores_by_hall() {
        assert!(score_memory("something", "fact") >= 0.6);
        assert!(score_memory("something", "event") >= 0.45);
        assert!(score_memory("something", "event") <= 0.55);
    }

    #[test]
    fn decision_keyword_boosts() {
        let no_kw = score_memory("something happened today", "fact");
        let one_kw = score_memory("Alice decided to use Clerk for auth", "fact");
        let two_kw = score_memory("Alice decided and chose to switch to Clerk", "fact");

        assert!(
            one_kw > no_kw,
            "one keyword should boost: {one_kw} > {no_kw}"
        );
        assert!(
            two_kw > one_kw,
            "two keywords should boost more: {two_kw} > {one_kw}"
        );
    }

    #[test]
    fn score_never_exceeds_one() {
        let score = score_memory(
            "decided this policy: never crash on error, the critical bug failed the blocker — \
             realized we must always fix issues, learned the insight from this breakthrough",
            "fact",
        );
        assert!((score - 1.0).abs() < f64::EPSILON || score <= 1.0);
    }

    #[test]
    fn score_produces_gradient_not_binary() {
        // Different keyword densities should produce different scores
        let scores: Vec<f64> = vec![
            score_memory("hello world", "event"),
            score_memory("fixed a small issue", "event"),
            score_memory("decided to fix the critical bug", "fact"),
            score_memory(
                "decided and chose to fix the critical crash bug, learned insight",
                "fact",
            ),
        ];

        // Each should be strictly greater than the previous
        for i in 1..scores.len() {
            assert!(
                scores[i] > scores[i - 1],
                "expected gradient: scores[{i}]={} > scores[{}]={}",
                scores[i],
                i - 1,
                scores[i - 1]
            );
        }
    }

    #[test]
    fn signal_score_distribution_is_smooth() {
        // Generate synthetic memories with varying keyword densities
        let test_cases = vec![
            ("A simple observation about the weather", "event"),
            ("The system status looks normal today", "event"),
            ("Noticed the latency is high during peak hours", "discovery"),
            ("Found that the API was slow", "discovery"),
            ("Decided to use PostgreSQL for storage", "fact"),
            ("We chose React and decided on TypeScript", "fact"),
            ("Critical bug crashed the production system", "fact"),
            (
                "Learned a breakthrough insight about caching strategy",
                "discovery",
            ),
            (
                "Always follow this rule: never deploy on Friday",
                "preference",
            ),
            (
                "Decided to fix the urgent blocker that crashed staging",
                "fact",
            ),
            ("Approved the policy change and switched providers", "fact"),
            ("The deadline is tomorrow, this is critical", "event"),
            (
                "We realized and learned that switching was the right choice",
                "discovery",
            ),
            (
                "Chose to reject the failed approach due to critical bugs",
                "fact",
            ),
            ("Must always require approval before deploy", "preference"),
        ];

        let mut buckets = [0usize; 5]; // 0.5-0.6, 0.6-0.7, 0.7-0.8, 0.8-0.9, 0.9-1.0
        for (content, hall) in &test_cases {
            let score = score_memory(content, hall);
            let idx = ((score - 0.5) * 10.0).floor() as usize;
            let idx = idx.min(4);
            buckets[idx] += 1;
        }

        let total = test_cases.len();
        // No single bucket should hold more than 40% of the mass
        for (i, &count) in buckets.iter().enumerate() {
            let pct = count as f64 / total as f64;
            assert!(
                pct <= 0.40,
                "bucket {i} (0.{}-0.{}) holds {:.0}% > 40% ({count}/{total})",
                5 + i,
                6 + i,
                pct * 100.0
            );
        }
    }
}
