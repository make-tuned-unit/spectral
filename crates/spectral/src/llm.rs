//! Built-in HTTP LLM client for OpenAI-compatible endpoints.
//!
//! Feature-gated behind `http-llm` (default-on). Disable with
//! `default-features = false` if you don't need it.

pub use spectral_tact::LlmClient;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// HTTP LLM client that speaks the OpenAI chat-completions protocol.
///
/// Works with OpenAI, Ollama, LiteLLM, and any other endpoint that
/// accepts `POST /v1/chat/completions` with the standard JSON shape.
///
/// ```
/// use spectral::llm::HttpLlmClient;
///
/// // OpenAI
/// let _client = HttpLlmClient::openai("sk-...");
///
/// // Local Ollama
/// let _client = HttpLlmClient::ollama("llama3");
///
/// // Any compatible endpoint
/// let _client = HttpLlmClient::new("http://localhost:8080", "my-model")
///     .with_api_key("secret");
/// ```
#[derive(Debug, Clone)]
pub struct HttpLlmClient {
    base_url: String,
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl HttpLlmClient {
    /// Construct a client for any OpenAI-compatible endpoint.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            model: model.into(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    /// Convenience constructor for OpenAI (`https://api.openai.com`).
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self::new("https://api.openai.com", "gpt-4o-mini").with_api_key(api_key)
    }

    /// Convenience constructor for a local Ollama server.
    pub fn ollama(model: impl Into<String>) -> Self {
        Self::new("http://localhost:11434", model)
    }

    /// Set the API key (for endpoints that require one).
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Set a custom timeout (default: 30 seconds).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("failed to build HTTP client");
        self
    }
}

impl LlmClient for HttpLlmClient {
    fn complete(
        &self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + '_>> {
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.0,
        });

        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        Box::pin(async move {
            let resp = req.send().await?;
            let status = resp.status();
            let text = resp.text().await?;

            if !status.is_success() {
                anyhow::bail!("LLM API error {status}: {text}");
            }

            let json: serde_json::Value = serde_json::from_str(&text)?;
            let content = json["choices"][0]["message"]["content"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("unexpected response shape: {text}"))?;

            Ok(content.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_constructor_defaults() {
        let c = HttpLlmClient::openai("sk-test");
        assert_eq!(c.base_url, "https://api.openai.com");
        assert_eq!(c.model, "gpt-4o-mini");
        assert_eq!(c.api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn ollama_constructor_defaults() {
        let c = HttpLlmClient::ollama("llama3");
        assert_eq!(c.base_url, "http://localhost:11434");
        assert_eq!(c.model, "llama3");
        assert!(c.api_key.is_none());
    }

    #[test]
    fn custom_endpoint_with_key() {
        let c = HttpLlmClient::new("http://my-proxy:8080", "custom-model").with_api_key("my-key");
        assert_eq!(c.base_url, "http://my-proxy:8080");
        assert_eq!(c.model, "custom-model");
        assert_eq!(c.api_key.as_deref(), Some("my-key"));
    }

    #[tokio::test]
    async fn mock_server_request_shape() {
        use wiremock::matchers::{body_string_contains, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_string_contains("test-model"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "hello from mock"}}]
            })))
            .mount(&server)
            .await;

        let client = HttpLlmClient::new(server.uri(), "test-model").with_api_key("test-key");
        let result = client.complete("test prompt").await.unwrap();
        assert_eq!(result, "hello from mock");
    }

    #[tokio::test]
    async fn mock_server_no_auth_header_when_no_key() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "ok"}}]
            })))
            .mount(&server)
            .await;

        let client = HttpLlmClient::new(server.uri(), "m");
        let result = client.complete("test").await.unwrap();
        assert_eq!(result, "ok");
    }

    #[tokio::test]
    async fn mock_server_error_propagates() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let client = HttpLlmClient::new(server.uri(), "m");
        let result = client.complete("test").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("500"));
    }
}
