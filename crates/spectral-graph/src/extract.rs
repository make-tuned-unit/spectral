//! LLM-based triple extraction from natural-language text.

use serde::{Deserialize, Serialize};

/// A triple extracted by the LLM from free text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedTriple {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

fn default_confidence() -> f64 {
    1.0
}

/// Builds extraction prompts and parses LLM responses.
pub struct ExtractionPrompt;

impl ExtractionPrompt {
    /// Build the extraction prompt given the text and the ontology's known predicates.
    pub fn build(text: &str, predicates: &[String]) -> String {
        let predicate_list = if predicates.is_empty() {
            "  (no predicates defined)".to_string()
        } else {
            predicates
                .iter()
                .map(|p| format!("  - {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            r#"Extract structured triples from the following text. Return ONLY a JSON object with a "triples" array. Each triple has "subject", "predicate", "object", and "confidence" (0.0-1.0) fields.

Rules:
- Only use predicates from this list:
{predicate_list}
- Do NOT invent predicates outside the list.
- Omit triples you cannot confidently extract.
- Subject and object should be entity names as they appear in the text.

Example output:
{{"triples": [{{"subject": "Alice", "predicate": "knows", "object": "Bob", "confidence": 0.95}}]}}

Text: {text}

JSON:"#
        )
    }

    /// Parse the LLM response into extracted triples.
    ///
    /// Tolerates surrounding prose by extracting the first JSON block.
    /// Skips malformed triples rather than failing the whole batch.
    pub fn parse(response: &str) -> Vec<ExtractedTriple> {
        let json_str = match extract_json_block(response) {
            Some(s) => s,
            None => return vec![],
        };

        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            triples: Vec<serde_json::Value>,
        }

        let wrapper: Wrapper = match serde_json::from_str(json_str) {
            Ok(w) => w,
            Err(_) => return vec![],
        };

        wrapper
            .triples
            .into_iter()
            .filter_map(|v| {
                let subject = v.get("subject")?.as_str()?.trim().to_string();
                let predicate = v.get("predicate")?.as_str()?.trim().to_string();
                let object = v.get("object")?.as_str()?.trim().to_string();

                if subject.is_empty() || predicate.is_empty() || object.is_empty() {
                    return None;
                }

                let confidence = v.get("confidence").and_then(|c| c.as_f64()).unwrap_or(1.0);

                Some(ExtractedTriple {
                    subject,
                    predicate,
                    object,
                    confidence,
                })
            })
            .collect()
    }
}

/// Extract the first JSON object `{...}` from a response that may contain prose.
fn extract_json_block(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_response() {
        let resp = r#"{"triples": [{"subject": "Alice", "predicate": "knows", "object": "Bob", "confidence": 0.9}]}"#;
        let triples = ExtractionPrompt::parse(resp);
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].subject, "Alice");
        assert_eq!(triples[0].predicate, "knows");
        assert_eq!(triples[0].object, "Bob");
        assert!((triples[0].confidence - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_with_surrounding_prose() {
        let resp = r#"Here are the triples: {"triples": [{"subject": "Alice", "predicate": "studies", "object": "Math", "confidence": 0.8}]} Hope that helps!"#;
        let triples = ExtractionPrompt::parse(resp);
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].subject, "Alice");
    }

    #[test]
    fn parse_missing_confidence_defaults() {
        let resp = r#"{"triples": [{"subject": "Alice", "predicate": "knows", "object": "Bob"}]}"#;
        let triples = ExtractionPrompt::parse(resp);
        assert_eq!(triples.len(), 1);
        assert!((triples[0].confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_garbage_returns_empty() {
        assert!(ExtractionPrompt::parse("not json at all").is_empty());
        assert!(ExtractionPrompt::parse("").is_empty());
        assert!(ExtractionPrompt::parse("{garbage}").is_empty());
    }

    #[test]
    fn parse_skips_malformed_triples() {
        let resp = r#"{"triples": [{"subject": "Alice", "predicate": "knows", "object": "Bob"}, {"subject": "", "predicate": "knows", "object": "Carol"}, {"bad": true}]}"#;
        let triples = ExtractionPrompt::parse(resp);
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].object, "Bob");
    }

    #[test]
    fn build_includes_predicates() {
        let prompt =
            ExtractionPrompt::build("Alice knows Bob", &["knows".into(), "studies".into()]);
        assert!(prompt.contains("- knows"));
        assert!(prompt.contains("- studies"));
        assert!(prompt.contains("Alice knows Bob"));
    }
}
