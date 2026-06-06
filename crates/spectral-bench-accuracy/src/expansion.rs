//! Pre-retrieval query expansion via LLM-generated synonym/domain terms.
//!
//! Generates additional search terms to bridge FTS vocabulary gaps
//! (e.g. "siblings" → "sisters brothers family members").

use anyhow::Result;

/// Configuration for query expansion.
#[derive(Debug, Clone)]
pub struct ExpansionConfig {
    /// Whether expansion is enabled.
    pub enabled: bool,
    /// Model to use for term generation (Haiku-class recommended).
    pub model: String,
    /// Base URL for API calls.
    pub base_url: String,
    /// API key.
    pub api_key: String,
    /// Maximum number of expansion terms to generate.
    pub max_terms: usize,
}

impl Default for ExpansionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: "claude-haiku-4-5-20251001".into(),
            base_url: "https://api.anthropic.com".into(),
            api_key: String::new(),
            max_terms: 10,
        }
    }
}

const EXPANSION_PROMPT: &str = "\
Generate {max_terms} single-word or short-phrase search terms that would help find \
the answer to this question in a text database. Focus on:
- Synonyms (e.g. siblings → sisters, brothers)
- Domain terms (e.g. bike expenses → helmet, tune-up, chain)
- Entity/ingredient names that might appear in relevant text
- Related concepts the user might have discussed

Question: {question}

Output ONLY the terms, one per line, no numbering, no explanation.";

/// Generate expansion terms for a question via LLM.
pub fn expand_query(question: &str, config: &ExpansionConfig) -> Result<String> {
    if !config.enabled {
        return Ok(question.to_string());
    }

    let prompt = EXPANSION_PROMPT
        .replace("{max_terms}", &config.max_terms.to_string())
        .replace("{question}", question);

    let client = reqwest::blocking::Client::new();
    let body = serde_json::json!({
        "model": config.model,
        "max_tokens": 200,
        "messages": [{"role": "user", "content": prompt}]
    });

    let resp = client
        .post(format!("{}/v1/messages", config.base_url))
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Expansion API returned {}: {}",
            status,
            body_text.chars().take(300).collect::<String>()
        ));
    }

    let json: serde_json::Value = resp.json()?;
    let terms_text = json["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // Combine original question with expansion terms
    let terms: Vec<&str> = terms_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(config.max_terms)
        .collect();

    if terms.is_empty() {
        return Ok(question.to_string());
    }

    Ok(format!("{} {}", question, terms.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_expansion_returns_original() {
        let config = ExpansionConfig::default();
        let result = expand_query("How many siblings?", &config).unwrap();
        assert_eq!(result, "How many siblings?");
    }
}
