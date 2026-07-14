//! Wing/hall classification for memory ingestion.
//!
//! Classifies raw text into a wing (project/domain) and hall (memory type)
//! using configurable regex rules.

use regex::Regex;

/// Classify text into a wing (project/domain).
///
/// First regex match wins. Returns `"general"` if no rule matches.
pub fn classify_wing(
    key: &str,
    content: &str,
    category: &str,
    rules: &[(Regex, String)],
) -> String {
    let blob = format!("{} {} {}", key, content, category).to_lowercase();
    for (pattern, wing) in rules {
        if pattern.is_match(&blob) {
            return wing.clone();
        }
    }
    "general".to_string()
}

/// Classify text into a hall (memory type).
///
/// First regex match wins. Returns `"event"` if no rule matches.
pub fn classify_hall(content: &str, rules: &[(Regex, String)]) -> String {
    let text = content.to_lowercase();
    for (pattern, hall) in rules {
        if pattern.is_match(&text) {
            return hall.clone();
        }
    }
    "event".to_string()
}

/// Default wing rule patterns as `(regex_pattern, wing_name)` string pairs.
///
/// Shared between ingest (compiled to `Regex`) and TACT retrieval (used as strings).
pub fn default_wing_rule_strings() -> Vec<(String, String)> {
    default_wing_rule_pairs()
        .into_iter()
        .map(|(p, w)| (p.to_string(), w.to_string()))
        .collect()
}

/// Default hall rule patterns as `(regex_pattern, hall_name)` string pairs.
pub fn default_hall_rule_strings() -> Vec<(String, String)> {
    default_hall_rule_pairs()
        .into_iter()
        .map(|(p, h)| (p.to_string(), h.to_string()))
        .collect()
}

fn default_wing_rule_pairs() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            r"alice|coffee|anniversary|colou?r|favourit|favorit|sons|noah|leo|carol-doe",
            "alice",
        ),
        (
            r"apollo|polymarket|strategy|weather|prediction|wager|trade",
            "apollo",
        ),
        (r"acme|widget|bob|recipe|cook|feast", "acme"),
        (r"charity|advocacy|grant|nonprofit|fundrais", "charity"),
        (r"vega|sales|purchase|commerce", "vega"),
        (r"travel|immigration|visa|permit", "travel"),
        (
            r"polaris|volunteer|plogging|litter|marathon|summit",
            "polaris",
        ),
        (
            r"task.runner|litellm|infrastructure|ollama|gemma|model.ladder",
            "infra",
        ),
    ]
}

fn default_hall_rule_pairs() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            r"decided|chose|switching to|using|will use|agreed|locked in",
            "fact",
        ),
        (
            r"remember|preference|favourit|favorit|likes|prefers",
            "preference",
        ),
        (
            r"learned|discovered|found that|realized|breakthrough",
            "discovery",
        ),
        (r"recommend|should|advice|suggest|try using", "advice"),
        // Durable personal facts the classifier previously dropped to "event".
        // Appended (first-match order preserved) so existing classifications are
        // untouched. FIRST-PERSON / STATE-ANCHORED to avoid matching transient
        // mentions of the same words ("the vegan cafe", "I never got the email",
        // "I like how it turned out") — precision measured in
        // classifier_precision_bench.
        // Health/dietary/medical constraints — the user describing THEMSELVES.
        (
            r"\bi'?m (\w+ly )?(allergic|vegetarian|vegan|diabetic|coeliac|celiac)\b|\bi am (\w+ly )?(allergic|vegetarian|vegan|diabetic|a vegetarian|a vegan|a diabetic|gluten[- ]?free)\b|\bi have (a |an )?[a-z]+ (allergy|intolerance)\b|\bmy (allergy|allergies|dietary)\b",
            "fact",
        ),
        // Family/identity — a durable STATE (relation, optional name, state verb),
        // not an event ("my son forgot his lunch").
        (
            r"\bmy (wife|husband|daughter|son|partner|mother|father)( \w+)? (is|are|works|lives|studies|goes to)\b",
            "fact",
        ),
        // Standing preferences — strong markers only (not bare "like/love").
        (r"\bi (\w+ly )?prefer\b|\bi'?d rather\b|\bmy favou?rite\b", "preference"),
        // Standing rules — directive framing after never/always, or explicit rule.
        (
            r"\b(never|always) (schedule|book|call|contact|email|send|use|run|deploy|share|give|forget|skip|miss)\b|\bmy rule is\b|\bas a rule\b|\bdo not ever\b|\bdon'?t ever\b",
            "rule",
        ),
    ]
}

/// Build default wing rules as compiled `Regex` (for ingestion classifier).
pub fn default_wing_rules() -> Vec<(Regex, String)> {
    default_wing_rule_pairs()
        .into_iter()
        .map(|(pat, wing)| {
            (
                Regex::new(pat).expect("invalid wing regex"),
                wing.to_string(),
            )
        })
        .collect()
}

/// Build default hall rules as compiled `Regex` (for ingestion classifier).
pub fn default_hall_rules() -> Vec<(Regex, String)> {
    default_hall_rule_pairs()
        .into_iter()
        .map(|(pat, hall)| {
            (
                Regex::new(pat).expect("invalid hall regex"),
                hall.to_string(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wing_personal() {
        let rules = default_wing_rules();
        assert_eq!(classify_wing("", "Alice likes coffee", "", &rules), "alice");
    }

    #[test]
    fn wing_apollo() {
        let rules = default_wing_rules();
        assert_eq!(
            classify_wing("", "apollo weather prediction", "", &rules),
            "apollo"
        );
    }

    #[test]
    fn wing_general_fallback() {
        let rules = default_wing_rules();
        assert_eq!(
            classify_wing("random", "hello world", "core", &rules),
            "general"
        );
    }

    #[test]
    fn wing_uses_key_and_category() {
        let rules = default_wing_rules();
        assert_eq!(
            classify_wing("alice_pref", "something", "core", &rules),
            "alice"
        );
    }

    #[test]
    fn hall_fact() {
        let rules = default_hall_rules();
        assert_eq!(
            classify_hall("Alice decided to use Clerk for auth", &rules),
            "fact"
        );
    }

    #[test]
    fn hall_preference() {
        let rules = default_hall_rules();
        assert_eq!(
            classify_hall("Alice prefers dark roast coffee", &rules),
            "preference"
        );
    }

    #[test]
    fn hall_event_fallback() {
        let rules = default_hall_rules();
        assert_eq!(classify_hall("deployed the new build", &rules), "event");
    }
}
