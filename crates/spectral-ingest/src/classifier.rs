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
            r"alice|coffee|anniversary|colou?r|favourit|favorit|sons|noah|leo|carol.doe",
            "alice",
        ),
        (
            r"apollo|polymarket|strategy|weather|prediction|wager|trade",
            "apollo",
        ),
        (r"acme|ladle|mel|recipe|cook|feast", "acme"),
        (r"love|lns|advocacy|grant|diana|eve|doe", "polaris"),
        (r"vega|cortex.sells|stripe|purchase", "vega"),
        (r"carol|immigration|example.co|visa|permit", "carol"),
        (
            r"polaris|wlr|plogging|litter|marathon|summit",
            "polaris",
        ),
        (
            r"task.runner|litellm|taskforge|infrastructure|ollama|gemma|model.ladder",
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
    fn wing_alice() {
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
