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

/// Count causal connectives, conditional structures, and multi-step reasoning
/// markers per sentence. Capped at 1.0.
pub fn causal_depth(content: &str) -> f64 {
    let lower = content.to_lowercase();

    // Causal connectives
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
        "this means",
        "in order to",
        "which means",
        "resulting in",
        "hence",
        "thus",
        "that's why",
        "for this reason",
    ];

    // Conditional structures
    let conditional_markers = [
        "if ",
        "when ",
        "given ",
        "assuming ",
        "unless ",
        "provided that",
        "in case",
        "otherwise",
    ];

    // Multi-step reasoning
    let sequence_markers = [
        "first,",
        "then ",
        "next,",
        "finally,",
        "step ",
        "after that",
        "following",
        "subsequently",
    ];

    let causal_count = causal_markers
        .iter()
        .filter(|m| lower.contains(**m))
        .count();
    let conditional_count = conditional_markers
        .iter()
        .filter(|m| lower.contains(**m))
        .count();
    let sequence_count = sequence_markers
        .iter()
        .filter(|m| lower.contains(**m))
        .count();

    let total = causal_count + conditional_count + sequence_count;
    let sentences = content
        .split(['.', '!', '?'])
        .filter(|s| !s.trim().is_empty())
        .count()
        .max(1);

    // Use sigmoid-like scaling: 1 marker per sentence → ~0.5, 2+ → higher
    let density = total as f64 / sentences as f64;
    (density / (density + 0.5)).min(1.0)
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

/// Detect explicit time anchors (dates, relative time, durations, time-of-day).
pub fn temporal_specificity(content: &str) -> f64 {
    let lower = content.to_lowercase();

    // Relative time references
    let relative_markers = [
        "yesterday",
        "today",
        "tomorrow",
        "last week",
        "this week",
        "next week",
        "last month",
        "this month",
        "next month",
        "last year",
        "this year",
        "next year",
        "recently",
        "soon",
        "later",
        "earlier",
        "ago",
        "in 2 ",
        "in 3 ",
        "in a few",
    ];

    // Time-of-day and scheduling
    let time_markers = [
        "morning",
        "afternoon",
        "evening",
        "tonight",
        "midnight",
        "deadline",
        "by the end of",
        "before the",
        "after the",
        "at noon",
        "scheduled",
        "meeting",
    ];

    // Day and month names
    let calendar_markers = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "january",
        "february",
        "march",
        "april",
        "may ",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];

    // Duration markers
    let duration_markers = [
        "for hours",
        "for days",
        "for weeks",
        "for months",
        "since last",
        "since the",
        "over the past",
        "during the",
        "throughout",
        "in the past",
    ];

    let relative_count = relative_markers
        .iter()
        .filter(|m| lower.contains(**m))
        .count();
    let time_count = time_markers.iter().filter(|m| lower.contains(**m)).count();
    let calendar_count = calendar_markers
        .iter()
        .filter(|m| lower.contains(**m))
        .count();
    let duration_count = duration_markers
        .iter()
        .filter(|m| lower.contains(**m))
        .count();

    // Date-like patterns: "2026-04-28", "04/28", "April 13", "May 5 2026"
    let date_regex_count = regex::Regex::new(
        r"\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}(?:/\d{2,4})?|\d{1,2}(?:st|nd|rd|th)?\s+\d{4}",
    )
    .unwrap()
    .find_iter(content)
    .count();

    // Clock time patterns: "3pm", "at 14:30", "10:00 AM"
    let clock_count = regex::Regex::new(r"\d{1,2}:\d{2}|\d{1,2}\s*(?:am|pm|AM|PM)")
        .unwrap()
        .find_iter(content)
        .count();

    let total = relative_count
        + time_count
        + calendar_count
        + duration_count
        + date_regex_count
        + clock_count;
    let sentences = content
        .split(['.', '!', '?'])
        .filter(|s| !s.trim().is_empty())
        .count()
        .max(1);

    // Sigmoid-like scaling: 1 marker per sentence → ~0.5, 2+ → higher
    let density = total as f64 / sentences as f64;
    (density / (density + 0.5)).min(1.0)
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
