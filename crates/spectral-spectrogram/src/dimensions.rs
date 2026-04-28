//! Cognitive dimension classifiers. Each function takes memory content
//! and returns a normalized score. Deterministic, no LLM required.

use crate::types::ActionType;

/// Count capitalized multi-word phrases and abbreviations, normalized by sqrt(content_length).
pub fn entity_density(content: &str) -> f64 {
    let mut count = 0usize;
    let words: Vec<&str> = content.split_whitespace().collect();
    for w in &words {
        let is_abbrev = w.len() >= 2
            && w.chars()
                .all(|c| c.is_ascii_uppercase() || matches!(c, '-'));
        let is_capitalized =
            w.chars().next().is_some_and(|c| c.is_uppercase()) && w.len() > 1 && !w.ends_with('.');
        if is_abbrev || is_capitalized {
            count += 1;
        }
    }
    let len = content.len().max(1) as f64;
    let raw = count as f64 / len.sqrt();
    raw.min(1.0)
}

/// Classify the action type by keyword matching.
pub fn action_type(content: &str) -> ActionType {
    let lower = content.to_lowercase();

    // Order matters: more specific patterns first.
    if lower.contains("decided")
        || lower.contains("chose")
        || lower.contains("going with")
        || lower.contains("locked in")
        || lower.contains("picked")
    {
        return ActionType::Decision;
    }
    if lower.contains("found that")
        || lower.contains("noticed")
        || lower.contains("realized")
        || lower.contains("discovered")
        || lower.contains("learned that")
    {
        return ActionType::Discovery;
    }
    if lower.contains("should")
        || lower.contains("recommend")
        || lower.contains("suggest")
        || lower.contains("advise")
    {
        return ActionType::Advice;
    }
    if lower.contains("thinking about")
        || lower.contains("considering")
        || lower.contains("reflecting")
        || lower.contains("wondering")
    {
        return ActionType::Reflection;
    }
    if lower.contains("build")
        || lower.contains("implement")
        || lower.contains("fix")
        || lower.contains("deploy")
        || lower.contains("ship")
    {
        return ActionType::Task;
    }
    ActionType::Observation
}

/// For decision-type memories, detect polarity. -1.0 = against, 0.0 = neutral, 1.0 = for.
pub fn decision_polarity(content: &str, at: ActionType) -> f64 {
    if at != ActionType::Decision {
        return 0.0;
    }
    let lower = content.to_lowercase();
    let positive = [
        "yes",
        "proceed",
        "approved",
        "going with",
        "chose",
        "picked",
    ]
    .iter()
    .filter(|p| lower.contains(**p))
    .count();
    let negative = [
        "no",
        "cancel",
        "rejected",
        "against",
        "not going",
        "decided against",
    ]
    .iter()
    .filter(|p| lower.contains(**p))
    .count();

    if positive == 0 && negative == 0 {
        return 0.0;
    }
    let raw = (positive as f64 - negative as f64) / (positive + negative) as f64;
    raw.clamp(-1.0, 1.0)
}

/// Count causal connectives per sentence. Capped at 1.0.
pub fn causal_depth(content: &str) -> f64 {
    let causal_markers = [
        "because",
        "therefore",
        "so that",
        "as a result",
        "leads to",
        "caused by",
        "due to",
        "since",
        "consequently",
    ];
    let lower = content.to_lowercase();
    let count = causal_markers
        .iter()
        .filter(|m| lower.contains(**m))
        .count();
    let sentences = content
        .split(['.', '!', '?'])
        .filter(|s| !s.trim().is_empty())
        .count()
        .max(1);
    (count as f64 / sentences as f64).min(1.0)
}

/// Positive minus negative sentiment words, normalized.
pub fn emotional_valence(content: &str) -> f64 {
    let lower = content.to_lowercase();
    let positive_words = [
        "great",
        "excited",
        "working",
        "success",
        "happy",
        "love",
        "perfect",
        "wonderful",
        "progress",
        "solved",
        "achieved",
    ];
    let negative_words = [
        "broken",
        "fail",
        "blocked",
        "frustrated",
        "hate",
        "terrible",
        "awful",
        "stuck",
        "problem",
        "error",
        "crash",
    ];
    let pos: usize = positive_words
        .iter()
        .map(|w| lower.matches(w).count())
        .sum();
    let neg: usize = negative_words
        .iter()
        .map(|w| lower.matches(w).count())
        .sum();

    if pos == 0 && neg == 0 {
        return 0.0;
    }
    let raw = (pos as f64 - neg as f64) / (pos + neg) as f64;
    raw.clamp(-1.0, 1.0)
}

/// Detect explicit time anchors (dates, relative time references).
pub fn temporal_specificity(content: &str) -> f64 {
    let lower = content.to_lowercase();
    let time_markers = [
        "yesterday",
        "today",
        "tomorrow",
        "last week",
        "this week",
        "next week",
        "last month",
        "deadline",
        "morning",
        "afternoon",
        "evening",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "january",
        "february",
        "march",
        "april",
    ];
    let count = time_markers.iter().filter(|m| lower.contains(**m)).count();

    // Also count date-like patterns (e.g. "2026-04-28", "04/28")
    let date_count = regex::Regex::new(r"\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}")
        .unwrap()
        .find_iter(content)
        .count();

    let total = count + date_count;
    let sentences = content
        .split(['.', '!', '?'])
        .filter(|s| !s.trim().is_empty())
        .count()
        .max(1);
    (total as f64 / sentences as f64).min(1.0)
}

/// Novelty relative to existing terms in the wing. Simple approach: count of
/// terms in the content that don't appear in the existing corpus text.
pub fn novelty(content: &str, existing_corpus: &str) -> f64 {
    let existing_lower = existing_corpus.to_lowercase();
    let words: Vec<&str> = content.split_whitespace().filter(|w| w.len() > 3).collect();
    if words.is_empty() {
        return 0.5;
    }
    let novel_count = words
        .iter()
        .filter(|w| !existing_lower.contains(&w.to_lowercase()))
        .count();
    (novel_count as f64 / words.len() as f64).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_type_decision() {
        assert_eq!(
            action_type("Decided to use Clerk for auth"),
            ActionType::Decision
        );
        assert_eq!(
            action_type("We chose PostgreSQL over MySQL"),
            ActionType::Decision
        );
    }

    #[test]
    fn action_type_discovery() {
        assert_eq!(
            action_type("Found that the cache was stale"),
            ActionType::Discovery
        );
        assert_eq!(
            action_type("Realized the API was deprecated"),
            ActionType::Discovery
        );
    }

    #[test]
    fn action_type_task() {
        assert_eq!(
            action_type("Need to build the auth module"),
            ActionType::Task
        );
    }

    #[test]
    fn action_type_observation_default() {
        assert_eq!(action_type("The sky is blue"), ActionType::Observation);
    }

    #[test]
    fn positive_valence() {
        let v = emotional_valence("Great progress on the project, we achieved our goal");
        assert!(v > 0.0, "expected positive valence, got {v}");
    }

    #[test]
    fn negative_valence() {
        let v = emotional_valence("The build is broken and we are stuck on this terrible problem");
        assert!(v < 0.0, "expected negative valence, got {v}");
    }

    #[test]
    fn entity_density_high_vs_low() {
        let high = entity_density("Alice and Bob worked at NASA on the DARPA project with IBM");
        let low = entity_density("the quick brown fox jumped over the lazy dog");
        assert!(
            high > low,
            "expected high density > low density: {high} vs {low}"
        );
    }

    #[test]
    fn causal_depth_detected() {
        let d = causal_depth("We changed it because the old approach leads to crashes. Therefore we picked the new design.");
        assert!(d > 0.0, "expected causal depth > 0, got {d}");
    }

    #[test]
    fn temporal_specificity_detected() {
        let t = temporal_specificity("Yesterday we deployed the fix. The deadline is 2026-04-30.");
        assert!(t > 0.0, "expected temporal specificity > 0, got {t}");
    }

    #[test]
    fn novelty_all_new() {
        let n = novelty(
            "quantum entanglement topological manifold",
            "the cat sat on the mat",
        );
        assert!(n > 0.5, "all-new terms should have high novelty, got {n}");
    }
}
