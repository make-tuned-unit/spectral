//! Actor LLM trait — given a question and retrieved memories, produce an answer.

use crate::report::TokenUsage;
use crate::retrieval::QuestionType;
use anyhow::Result;

/// Actor that synthesizes an answer from retrieved memories.
pub trait Actor: Send + Sync {
    /// Given a question, the question's date context, retrieved memories, and
    /// classified question shape, produce an answer and optional API token usage.
    fn answer(
        &self,
        question: &str,
        question_date: &str,
        memories: &[String],
        shape: QuestionType,
    ) -> Result<(String, Option<TokenUsage>)>;
    /// Identifier for the report.
    fn name(&self) -> &str;
}

/// Extract token usage from an Anthropic API response JSON.
fn extract_usage(json: &serde_json::Value) -> Option<TokenUsage> {
    let usage = json.get("usage")?;
    Some(TokenUsage {
        input_tokens: usage.get("input_tokens").and_then(|v| v.as_u64()),
        output_tokens: usage.get("output_tokens").and_then(|v| v.as_u64()),
    })
}

/// Extract the assistant's text from a Messages API response. Thinking-enabled
/// models (e.g. sonnet-5) return a `thinking` block as `content[0]` and the
/// answer as a later `text` block, so we scan for the first block with a `text`
/// field rather than assuming `content[0]`.
pub(crate) fn extract_text(json: &serde_json::Value) -> Option<String> {
    json.get("content")?
        .as_array()?
        .iter()
        .find_map(|block| {
            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                block.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        // Fallback: any block that happens to carry a text field.
        .or_else(|| {
            json["content"]
                .as_array()?
                .iter()
                .find_map(|b| b.get("text").and_then(|t| t.as_str()).map(String::from))
        })
}

/// Actor that calls the Anthropic Messages API.
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

    /// Send a pre-built request body to the Anthropic API and return the text + usage.
    pub fn call_raw(
        &self,
        body: &serde_json::Value,
    ) -> anyhow::Result<(String, Option<TokenUsage>)> {
        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(body)
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(anyhow::anyhow!(
                "API returned {}: {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }

        let json: serde_json::Value = resp.json()?;
        let usage = extract_usage(&json);
        let text = extract_text(&json)
            .ok_or_else(|| anyhow::anyhow!("Response missing a text block"))?;
        Ok((text, usage))
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
    fn answer(
        &self,
        question: &str,
        question_date: &str,
        memories: &[String],
        shape: QuestionType,
    ) -> Result<(String, Option<TokenUsage>)> {
        let memories_text = memories.join("\n");
        let template = shape.prompt_content();
        let prompt = template
            .replace("{question_date}", question_date)
            .replace("{memories_text}", &memories_text)
            .replace("{question}", question);

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            // Deterministic (greedy) decoding: an eval/A-B harness must pin
            // temperature or sampling noise swamps the effect under test. An
            // unpinned (=1.0) actor made a fetch_mult A/B inconclusive on n=30.
            "temperature": 0,
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
        let usage = extract_usage(&json);
        let text = extract_text(&json).ok_or_else(|| {
            anyhow::anyhow!(
                "Actor response missing a text block: {}",
                serde_json::to_string(&json).unwrap_or_default()
            )
        })?;
        Ok((text, usage))
    }

    fn name(&self) -> &str {
        &self.model
    }
}

/// Actor that calls an OpenAI-compatible `/v1/chat/completions` endpoint — for
/// driving a LOCAL model (ollama, llama.cpp server, LM Studio, vLLM) so the
/// accuracy loop runs fully on-device, no cloud dependency. Point `base_url` at
/// the local server (e.g. `http://localhost:11434` for ollama).
pub struct OpenAiActor {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::blocking::Client,
}

impl OpenAiActor {
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            api_key,
            model,
            base_url,
            client: reqwest::blocking::Client::new(),
        }
    }
}

impl Actor for OpenAiActor {
    fn answer(
        &self,
        question: &str,
        question_date: &str,
        memories: &[String],
        shape: QuestionType,
    ) -> Result<(String, Option<TokenUsage>)> {
        let memories_text = memories.join("\n");
        let prompt = shape
            .prompt_content()
            .replace("{question_date}", question_date)
            .replace("{memories_text}", &memories_text)
            .replace("{question}", question);

        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 4096,
            "temperature": 0,
            "stream": false,
        });

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let b = resp.text().unwrap_or_default();
            return Err(anyhow::anyhow!(
                "OpenAI-compat actor returned {}: {}",
                status,
                b.chars().take(500).collect::<String>()
            ));
        }

        let json: serde_json::Value = resp.json()?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "OpenAI-compat response missing choices[0].message.content: {}",
                    serde_json::to_string(&json).unwrap_or_default()
                )
            })?
            .to_string();
        let usage = json.get("usage").map(|u| TokenUsage {
            input_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()),
            output_tokens: u.get("completion_tokens").and_then(|v| v.as_u64()),
        });
        Ok((text, usage))
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
        _shape: QuestionType,
    ) -> Result<(String, Option<TokenUsage>)> {
        Ok((self.response.clone(), None))
    }

    fn name(&self) -> &str {
        "mock"
    }
}
