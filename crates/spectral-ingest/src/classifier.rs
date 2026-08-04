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
    // DELIBERATELY EMPTY.
    //
    // A wing is a *topic area* — a deployment's own projects and life areas.
    // That is consumer domain knowledge, and the library has no way to know it.
    //
    // This list previously shipped example-scenario fixtures as the default:
    //
    //     alice|coffee|anniversary|favourit|noah|leo|carol-doe  -> alice
    //     apollo|polymarket|strategy|weather|wager|trade        -> apollo
    //     acme|widget|bob|recipe|cook|feast                     -> acme
    //     charity, vega, travel, polaris, infra                 -> ...
    //
    // Those are not a taxonomy. They are demo data, and they did real harm:
    // in a live 1,738-memory brain they had captured **46 memories into
    // `apollo`, 18 into `alice`, 17 into `acme`, 16 into `polaris`** by keyword
    // collision — genuine content filed into fictional topic areas, sitting
    // beside the consumer's real wings (`henry-infra`, `permagent`, `getladle`,
    // `grocery-savings-planner`, ...).
    //
    // With no rules, `classify_wing` returns `"general"`: no taxonomy supplied,
    // no taxonomy invented. Consumers declare their own via
    // `BrainConfig::wing_rules`, which is the supported path and demonstrably
    // works — 46.5% of real queries name a real wing when the taxonomy is real,
    // versus 11.4% under the fixtures.
    //
    // Wings are NOT auto-derivable from corpus statistics: measured, coverage
    // and discrimination trade off with no workable point. See
    // `docs/internal/wing-taxonomy-2026-08-03.md`.
    Vec::new()
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
        (
            r"\bi (\w+ly )?prefer\b|\bi'?d rather\b|\bmy favou?rite\b",
            "preference",
        ),
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
    fn no_default_wing_taxonomy_is_invented() {
        // The library ships NO wing rules. A wing is deployment knowledge, and
        // inventing one filed real content into fictional topic areas — 46
        // memories into `apollo`, 18 into `alice` in a live brain.
        let rules = default_wing_rules();
        assert!(
            rules.is_empty(),
            "the library must not ship a wing taxonomy"
        );
        for content in [
            "Alice likes coffee",
            "apollo weather prediction",
            "acme widget recipe",
            "polaris volunteer marathon",
        ] {
            assert_eq!(
                classify_wing("", content, "", &rules),
                "general",
                "fixture wing leaked back in for {content:?}"
            );
        }
    }

    #[test]
    fn consumer_supplied_wings_are_the_supported_path() {
        // What a real deployment does: declare its own project areas. Measured
        // on the real Permagent brain, 46.5% of real queries name a real wing
        // under a real taxonomy (vs 11.4% under the fixtures).
        let rules = compile(&[
            (r"henry-infra|task runner|deploy", "henry-infra"),
            (r"getladle|ladle", "getladle"),
        ]);
        assert_eq!(
            classify_wing("", "the task runner deploy failed", "", &rules),
            "henry-infra"
        );
        assert_eq!(
            classify_wing("", "ladle onboarding", "", &rules),
            "getladle"
        );
        // Unmatched content still falls back honestly.
        assert_eq!(
            classify_wing("", "unrelated content", "", &rules),
            "general"
        );
    }

    /// Helper mirroring how `BrainConfig::wing_rules` are compiled.
    fn compile(pairs: &[(&str, &str)]) -> Vec<(Regex, String)> {
        pairs
            .iter()
            .map(|(p, w)| (Regex::new(p).unwrap(), w.to_string()))
            .collect()
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
    fn wing_matching_still_considers_key_and_category() {
        // The key and category are part of the match blob — a consumer rule can
        // route on them, not only on content.
        let rules = compile(&[(r"permagent", "permagent")]);
        assert_eq!(
            classify_wing("permagent_pref", "something", "core", &rules),
            "permagent"
        );
        assert_eq!(
            classify_wing("k", "something", "permagent", &rules),
            "permagent"
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
