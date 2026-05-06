//! Actor LLM trait — given a question and retrieved memories, produce an answer.

use anyhow::Result;

/// Actor that synthesizes an answer from retrieved memories.
pub trait Actor: Send + Sync {
    /// Given a question, the question's date context, and retrieved memories, produce an answer.
    fn answer(&self, question: &str, question_date: &str, memories: &[String]) -> Result<String>;
    /// Identifier for the report.
    fn name(&self) -> &str;
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
             \n\
             Instructions:\n\
             1. For counting, listing, or ordering questions: the answer may be distributed across \
             multiple distinct conversation sessions. Each session has a unique prefix in the memory \
             keys (the part before the first colon). Identify each distinct session prefix in the \
             retrieved memories, then enumerate items from EVERY session before counting or ordering. \
             Do not stop after finding items in one or two sessions.\n\
             2. For questions about your current or most recent X: identify the most recent memory \
             mentioning X and treat that value as definitive, even if older memories mention different \
             values.\n\
             3. When information appears partial across memories, attempt synthesis from the available \
             evidence rather than saying \"I don't know.\" Only respond with \"I don't know\" when no \
             memory contains relevant content for the question.\n\
             4. When the question asks whether something happened (e.g., \"did I mention X?\"), and X \
             is not present in any memory, state that clearly and note what IS present in the memories \
             (e.g., \"You mentioned Y but not X\").\n\
             5. When multiple distinct entities or locations match the question (e.g., multiple stores, \
             multiple vehicles), do not pick the first one mentioned. Identify which entity the question \
             is specifically asking about and verify against the most relevant memories before answering.\n\
             6. For questions requiring arithmetic across memories (computing differences, sums, ages, \
             totals): identify the relevant numerical values from the memories and perform the calculation \
             explicitly. Show the values used and the result.\n\
             \n\
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
