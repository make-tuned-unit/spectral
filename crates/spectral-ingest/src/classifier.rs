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

/// Build default wing rules matching production `memory_tagger.py`.
pub fn default_wing_rules() -> Vec<(Regex, String)> {
    let rules: Vec<(&str, &str)> = vec![
        (
            r"jesse|coffee|anniversary|colou?r|favourit|favorit|sons|rowan|jude|sophie.sharratt",
            "jesse",
        ),
        (
            r"polybot|polymarket|strategy|weather|prediction|wager|trade",
            "polybot",
        ),
        (r"getladle|ladle|mel|recipe|cook|feast", "getladle"),
        (r"love|lns|advocacy|grant|dennis|jill|barkhouse", "love-ns"),
        (r"permagent|henry.sells|stripe|purchase", "permagent"),
        (r"sophie|immigration|north.star|visa|permit", "sophie"),
        (
            r"worldlitterrun|wlr|plogging|litter|marathon|bluenose",
            "worldlitterrun",
        ),
        (
            r"task.runner|litellm|zeroclaw|infrastructure|ollama|gemma|model.ladder",
            "henry-infra",
        ),
    ];
    rules
        .into_iter()
        .map(|(pat, wing)| {
            (
                Regex::new(pat).expect("invalid wing regex"),
                wing.to_string(),
            )
        })
        .collect()
}

/// Build default hall rules matching production `memory_tagger.py`.
pub fn default_hall_rules() -> Vec<(Regex, String)> {
    let rules: Vec<(&str, &str)> = vec![
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
    ];
    rules
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
    fn wing_jesse() {
        let rules = default_wing_rules();
        assert_eq!(classify_wing("", "Jesse likes coffee", "", &rules), "jesse");
    }

    #[test]
    fn wing_polybot() {
        let rules = default_wing_rules();
        assert_eq!(
            classify_wing("", "polybot weather prediction", "", &rules),
            "polybot"
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
            classify_wing("jesse_pref", "something", "core", &rules),
            "jesse"
        );
    }

    #[test]
    fn hall_fact() {
        let rules = default_hall_rules();
        assert_eq!(
            classify_hall("Jesse decided to use Clerk for auth", &rules),
            "fact"
        );
    }

    #[test]
    fn hall_preference() {
        let rules = default_hall_rules();
        assert_eq!(
            classify_hall("Jesse prefers dark roast coffee", &rules),
            "preference"
        );
    }

    #[test]
    fn hall_event_fallback() {
        let rules = default_hall_rules();
        assert_eq!(classify_hall("deployed the new build", &rules), "event");
    }
}
