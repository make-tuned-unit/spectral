//! Signal score computation for memory ingestion.
//!
//! Scores memories by cognitive significance (0.0–1.0). Matches the
//! production `signal_scorer.py` algorithm exactly.

/// Compute signal_score for a memory. Returns 0.0–1.0.
pub fn score_memory(content: &str, hall: &str) -> f64 {
    let hall_lower = hall.to_lowercase();
    let mut score: f64 = match hall_lower.as_str() {
        "fact" => 0.7,
        "discovery" => 0.65,
        "preference" => 0.6,
        "advice" => 0.55,
        "event" => 0.5,
        _ => 0.5,
    };

    let content_lower = content.to_lowercase();

    if has_any(
        &content_lower,
        &["decided", "chose", "switched", "approved", "rejected"],
    ) {
        score += 0.15;
    }
    if has_any(
        &content_lower,
        &["error", "bug", "failed", "broke", "crash"],
    ) {
        score += 0.1;
    }
    if has_any(
        &content_lower,
        &["learned", "realized", "breakthrough", "insight"],
    ) {
        score += 0.1;
    }
    if has_any(
        &content_lower,
        &["always", "never", "rule", "policy", "must"],
    ) {
        score += 0.1;
    }
    if has_any(
        &content_lower,
        &["deadline", "urgent", "critical", "blocker"],
    ) {
        score += 0.1;
    }

    score.min(1.0)
}

fn has_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_base_score() {
        assert!((score_memory("something happened", "fact") - 0.7).abs() < 0.001);
    }

    #[test]
    fn event_base_score() {
        assert!((score_memory("something happened", "event") - 0.5).abs() < 0.001);
    }

    #[test]
    fn decision_boost() {
        let score = score_memory("Alice decided to use Clerk for auth", "fact");
        assert!(
            (score - 0.85).abs() < 0.001,
            "fact(0.7) + decided(0.15) = 0.85, got {score}"
        );
    }

    #[test]
    fn multiple_boosts_clamped() {
        let score = score_memory("decided this policy: never crash on error", "fact");
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn unknown_hall_defaults_to_half() {
        assert!((score_memory("hello", "custom_hall") - 0.5).abs() < 0.001);
    }

    #[test]
    fn discovery_with_insight_boost() {
        let score = score_memory("I learned a breakthrough insight", "discovery");
        assert!((score - 0.75).abs() < 0.001);
    }
}
