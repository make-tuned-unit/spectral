//! Wing and hall classification — regex-based keyword matching.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use regex::Regex;

/// Compile-once cache for classification patterns.
///
/// `TactConfig` stores wing/hall rules as pattern *strings*, and classification
/// runs on every TACT retrieval — which is every cascade recall. Compiling each
/// pattern per call made classification cost ~0.28 ms, entirely in
/// `Regex::new`. Patterns come from configuration and are a small fixed set, so
/// they are cached by pattern text.
///
/// A pattern that fails to compile is cached as `None` and skipped, preserving
/// the previous `if let Ok(re)` behaviour exactly (including not erroring out).
fn compiled(pattern: &str) -> Option<Regex> {
    static CACHE: OnceLock<RwLock<HashMap<String, Option<Regex>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| RwLock::new(HashMap::new()));

    if let Ok(read) = cache.read() {
        if let Some(hit) = read.get(pattern) {
            // `Regex` is internally reference-counted, so this clone is cheap.
            return hit.clone();
        }
    }

    let built = Regex::new(pattern).ok();
    if let Ok(mut write) = cache.write() {
        write.insert(pattern.to_string(), built.clone());
    }
    built
}

/// Detect the wing (project/domain) from a query string.
pub fn detect_wing(msg: &str, rules: &[(String, String)]) -> Option<String> {
    let lower = msg.to_lowercase();
    for (pattern, wing) in rules {
        if let Some(re) = compiled(pattern) {
            if re.is_match(&lower) {
                return Some(wing.clone());
            }
        }
    }
    None
}

/// Detect the hall (knowledge type) from a query string.
pub fn detect_hall(msg: &str, rules: &[(String, String)]) -> Option<String> {
    let lower = msg.to_lowercase();
    for (pattern, hall) in rules {
        if let Some(re) = compiled(pattern) {
            if re.is_match(&lower) {
                return Some(hall.clone());
            }
        }
    }
    None
}

/// Extract query terms for overlap boosting. Filters out terms <= 2 chars.
pub fn extract_query_terms(msg: &str) -> Vec<String> {
    // Static pattern: compile once, not once per query.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[a-z0-9]+").unwrap());
    re.find_iter(&msg.to_lowercase())
        .map(|m| m.as_str().to_string())
        .filter(|t| t.len() > 2)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hall_rules() -> Vec<(String, String)> {
        vec![
            (r"decided|chose|decision".into(), "fact".into()),
            (
                r"learned|discovered|breakthrough".into(),
                "discovery".into(),
            ),
        ]
    }

    #[test]
    fn hall_detected() {
        let rules = sample_hall_rules();
        assert_eq!(
            detect_hall("what was the auth decision?", &rules),
            Some("fact".into())
        );
    }

    #[test]
    fn hall_none_when_no_match() {
        let rules = sample_hall_rules();
        assert_eq!(detect_hall("hello world", &rules), None);
    }

    #[test]
    fn query_terms_extracted() {
        let terms = extract_query_terms("what is the auth decision for acme?");
        assert!(terms.contains(&"auth".to_string()));
        assert!(terms.contains(&"decision".to_string()));
        assert!(!terms.contains(&"is".to_string()));
    }
}
