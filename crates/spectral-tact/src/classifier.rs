//! Wing and hall classification — regex-based keyword matching.

use regex::Regex;

/// Detect the wing (project/domain) from a query string.
pub fn detect_wing(msg: &str, rules: &[(String, String)]) -> Option<String> {
    let lower = msg.to_lowercase();
    for (pattern, wing) in rules {
        if let Ok(re) = Regex::new(pattern) {
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
        if let Ok(re) = Regex::new(pattern) {
            if re.is_match(&lower) {
                return Some(hall.clone());
            }
        }
    }
    None
}

/// Extract query terms for overlap boosting. Filters out terms <= 2 chars.
pub fn extract_query_terms(msg: &str) -> Vec<String> {
    let re = Regex::new(r"[a-z0-9]+").unwrap();
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
        let terms = extract_query_terms("what is the auth decision for getladle?");
        assert!(terms.contains(&"auth".to_string()));
        assert!(terms.contains(&"decision".to_string()));
        assert!(!terms.contains(&"is".to_string()));
    }
}
