//! Actor LLM trait — given a question and retrieved memories, produce an answer.

use anyhow::Result;

/// Actor that synthesizes an answer from retrieved memories.
pub trait Actor: Send + Sync {
    /// Given a question, the question's date context, and retrieved memories, produce an answer.
    fn answer(&self, question: &str, question_date: &str, memories: &[String]) -> Result<String>;
    /// Identifier for the report.
    fn name(&self) -> &str;
}

/// Actor that calls the Anthropic Messages API (or compatible endpoint).
pub struct AnthropicActor {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::blocking::Client,
}

impl AnthropicActor {
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            api_key,
            model,
            base_url,
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
        Ok(Self::new(
            api_key,
            "claude-sonnet-4-6".into(),
            "https://api.anthropic.com".into(),
        ))
    }
}

impl Actor for AnthropicActor {
    fn answer(&self, question: &str, question_date: &str, memories: &[String]) -> Result<String> {
        let memories_text = memories.join("\n");
        let prompt = format!(
            "You are answering a question based on a long conversation history.\n\
             Today's date is {question_date}.\n\
             Below are memories retrieved from the conversation, each prefixed \
             with the date it was created.\n\
             Answer the question accurately based ONLY on these memories.\n\
             If the answer cannot be determined from the memories, say \"I don't know.\"\n\n\
             Memories:\n{memories_text}\n\n\
             Question: {question}\n\n\
             Answer:"
        );

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": prompt}]
        });

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Actor API returned {}: {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        let json: serde_json::Value = resp.json()?;
        let text = json["content"][0]["text"]
            .as_str()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Actor response missing content[0].text: {}",
                    serde_json::to_string(&json).unwrap_or_default()
                )
            })?
            .to_string();
        Ok(text)
    }

    fn name(&self) -> &str {
        &self.model
    }
}

/// Mock actor for testing. Returns a canned response.
pub struct MockActor {
    response: String,
}

impl MockActor {
    pub fn new(response: &str) -> Self {
        Self {
            response: response.into(),
        }
    }
}

impl Actor for MockActor {
    fn answer(
        &self,
        _question: &str,
        _question_date: &str,
        _memories: &[String],
    ) -> Result<String> {
        Ok(self.response.clone())
    }

    fn name(&self) -> &str {
        "mock"
    }
}
