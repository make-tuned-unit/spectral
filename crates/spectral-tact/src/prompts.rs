//! Prompt templates for optional LLM-based classification.

/// Prompt template for wing (domain) classification.
pub fn wing_classification_prompt(query: &str, available_wings: &[&str]) -> String {
    let wings_list = available_wings.join(", ");
    format!(
        r#"Classify the following query into exactly one project/domain category.

Available categories: {wings_list}

If the query does not clearly belong to any category, respond with "none".
Respond with ONLY the category name, nothing else.

Query: {query}"#
    )
}

/// Prompt template for hall (knowledge type) classification.
pub fn hall_classification_prompt(query: &str) -> String {
    format!(
        r#"Classify the following query into exactly one knowledge type.

Available types:
- fact: Decisions, authoritative choices, settled questions
- preference: Personal preferences, likes, favorites
- discovery: Learned information, breakthroughs, new findings
- advice: Recommendations, suggestions, best practices

If the query does not clearly match any type, respond with "none".
Respond with ONLY the type name, nothing else.

Query: {query}"#
    )
}

/// Parse an LLM classification response.
pub fn parse_classification_response<'a>(
    response: &str,
    valid_values: &[&'a str],
) -> Option<&'a str> {
    let cleaned = response
        .trim()
        .trim_matches('"')
        .trim_matches('`')
        .trim_matches('*')
        .to_lowercase();

    valid_values.iter().find(|&&v| cleaned == v).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wing_prompt_includes_categories() {
        let prompt = wing_classification_prompt("auth setup", &["apollo", "commerce"]);
        assert!(prompt.contains("apollo, commerce"));
        assert!(prompt.contains("auth setup"));
    }

    #[test]
    fn parse_clean_response() {
        let valid = &["fact", "preference", "discovery", "advice"];
        assert_eq!(parse_classification_response("fact", valid), Some("fact"));
        assert_eq!(parse_classification_response("none", valid), None);
    }

    #[test]
    fn parse_response_with_quotes() {
        let valid = &["fact", "preference"];
        assert_eq!(
            parse_classification_response("  \"fact\"  ", valid),
            Some("fact")
        );
        assert_eq!(
            parse_classification_response("`discovery`", &["discovery"]),
            Some("discovery")
        );
    }
}
