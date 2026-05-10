//! LLM client implementation

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::Client;

use super::types::{ChatCompletionRequest, ChatCompletionResponse};
use crate::config::AppConfig;

#[derive(Debug, Clone)]
pub struct LlmClient {
    http: Client,
    base_url: String,
    model: String,
    max_retries: u32,
}

impl LlmClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            model: model.into(),
            max_retries: 3,
        })
    }

    pub fn from_config(config: &AppConfig) -> Result<Self> {
        if config.provider != "lmstudio" {
            bail!("unsupported LLM provider: {}", config.provider);
        }

        Self::new(&config.base_url, &config.model)
    }

    pub fn with_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Get the model name
    pub fn model(&self) -> &str {
        &self.model
    }

    async fn generate_with_retry(
        &self,
        request: &ChatCompletionRequest<'_>,
    ) -> Result<ChatCompletionResponse> {
        let mut last_error = None;

        for attempt in 0..=self.max_retries {
            let response = self
                .http
                .post(format!("{}/v1/chat/completions", self.base_url))
                .json(request)
                .send()
                .await;

            match response {
                Ok(resp) => match resp.error_for_status() {
                    Ok(ok_resp) => match ok_resp.json::<ChatCompletionResponse>().await {
                        Ok(parsed) => return Ok(parsed),
                        Err(e) => {
                            last_error =
                                Some(anyhow::anyhow!("Failed to parse LLM response: {}", e));
                        }
                    },
                    Err(e) => {
                        last_error = Some(anyhow::anyhow!("LLM endpoint returned error: {}", e));
                    }
                },
                Err(e) => {
                    last_error = Some(anyhow::anyhow!("Failed to call LLM endpoint: {}", e));
                }
            }

            if attempt < self.max_retries {
                let delay = Duration::from_millis(100 * 2_u64.pow(attempt));
                tokio::time::sleep(delay).await;
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Max retries exceeded")))
    }

    pub async fn generate(&self, prompt: &str) -> Result<String> {
        let request = ChatCompletionRequest {
            model: &self.model,
            messages: vec![super::types::ChatMessage {
                role: "user",
                content: prompt,
            }],
            stream: false,
        };

        let response = self.generate_with_retry(&request).await?;

        let Some(choice) = response.choices.into_iter().next() else {
            bail!("LLM response did not include any choices");
        };

        Ok(choice.message.content)
    }

    pub async fn generate_stream<F>(&self, prompt: &str, mut callback: F) -> Result<String>
    where
        F: FnMut(&str),
    {
        let request = ChatCompletionRequest {
            model: &self.model,
            messages: vec![super::types::ChatMessage {
                role: "user",
                content: prompt,
            }],
            stream: true,
        };

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&request)
            .send()
            .await
            .with_context(|| format!("failed to call LLM endpoint at {}", self.base_url))?
            .error_for_status()
            .context("LLM endpoint returned an error status")?;

        let bytes = response
            .bytes()
            .await
            .context("failed to read response body")?;
        let text = String::from_utf8_lossy(&bytes);

        let mut full_content = String::new();

        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    continue;
                }
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                    && let Some(content) = parsed
                        .get("choices")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("delta"))
                        .and_then(|d| d.get("content"))
                        .and_then(|c| c.as_str())
                {
                    callback(content);
                    full_content.push_str(content);
                }
            }
        }

        Ok(full_content)
    }
}
