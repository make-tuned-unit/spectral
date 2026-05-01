//! Actor LLM trait — given a question and retrieved memories, produce an answer.

use anyhow::Result;

/// Actor that synthesizes an answer from retrieved memories.
pub trait Actor: Send + Sync {
    /// Given a question and retrieved memories, produce an answer.
    fn answer(&self, question: &str, memories: &[String]) -> Result<String>;
    /// Identifier for the report.
    fn name(&self) -> &str;
}

/// Actor that calls the Anthropic Messages API.
pub struct AnthropicActor {
    api_key: String,
    model: String,
    client: reqwest::blocking::Client,
}

impl AnthropicActor {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
        Ok(Self::new(api_key, "claude-sonnet-4-5-20250514".into()))
    }
}

impl Actor for AnthropicActor {
    fn answer(&self, question: &str, memories: &[String]) -> Result<String> {
        let memories_text = memories.join("\n");
        let prompt = format!(
            "You are answering a question based on a long conversation history.\n\
             Below are memories retrieved from the conversation.\n\
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
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()?;

        let json: serde_json::Value = resp.json()?;
        let text = json["content"][0]["text"]
            .as_str()
            .unwrap_or("")
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
    fn answer(&self, _question: &str, _memories: &[String]) -> Result<String> {
        Ok(self.response.clone())
    }

    fn name(&self) -> &str {
        "mock"
    }
}
