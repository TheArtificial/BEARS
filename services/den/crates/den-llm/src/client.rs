use std::time::{Duration, Instant};

use futures::{Stream, StreamExt};
use reqwest::Response;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use den_core::{config::Config, DenError};

/// Bifrost expects `provider/model`; bare OpenAI-style ids get an `openai/` prefix.
pub fn normalize_llm_model_handle(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return "openai/gpt-4o-mini".to_string();
    }
    if trimmed.contains('/') {
        trimmed.to_string()
    } else {
        format!("openai/{trimmed}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ChatToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<LlmToolDefinition>,
    pub stream: bool,
    pub tool_choice: Option<Value>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

impl ChatCompletionRequest {
    pub fn to_body(&self) -> Value {
        let tools: Vec<Value> = self
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect();
        let mut body = json!({
            "model": self.model,
            "messages": self.messages,
            "stream": self.stream,
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        if let Some(tool_choice) = &self.tool_choice {
            body["tool_choice"] = tool_choice.clone();
        }
        if let Some(temperature) = self.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = self.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        body
    }
}

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
}

impl LlmClient {
    pub fn new(config: &Config) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client for llm");
        Self {
            http,
            base_url: config.llm_api_url.trim_end_matches('/').to_string(),
            api_key: config.llm_api_key.clone(),
            default_model: normalize_llm_model_handle(&config.default_llm_model),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.base_url.is_empty()
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    pub fn resolve_model(&self, requested: Option<&str>) -> String {
        let raw = requested
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or(&self.default_model);
        normalize_llm_model_handle(raw)
    }

    pub async fn chat_completions_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<Response, DenError> {
        if !self.is_enabled() {
            return Err(DenError::System(
                "LLM API is not configured (set LLM_API_URL or BIFROST_BASE_URL)".to_string(),
            ));
        }
        let url = format!("{}/chat/completions", self.base_url);
        tracing::info!(
            model = %request.model,
            message_count = request.messages.len(),
            tool_count = request.tools.len(),
            stream = request.stream,
            llm_url = %url,
            "LLM chat/completions request starting"
        );
        let started = Instant::now();
        let mut req = self.http.post(&url).json(&request.to_body());
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(
                    model = %request.model,
                    duration_ms = started.elapsed().as_millis(),
                    error = %e,
                    "LLM chat/completions request failed"
                );
                DenError::System(format!("LLM chat/completions request failed: {e}"))
            })?;
        let http_status = resp.status().as_u16();
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                model = %request.model,
                http_status,
                duration_ms = started.elapsed().as_millis(),
                response_body_len = text.len(),
                "LLM chat/completions returned error status"
            );
            return Err(DenError::System(format!(
                "LLM chat/completions HTTP {status}: {text}"
            )));
        }
        tracing::info!(
            model = %request.model,
            http_status,
            duration_ms = started.elapsed().as_millis(),
            "LLM chat/completions response received"
        );
        Ok(resp)
    }

    pub async fn chat_completions_byte_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<impl Stream<Item = Result<bytes::Bytes, DenError>> + Send + Unpin, DenError>
    {
        let resp = self.chat_completions_stream(request).await?;
        Ok(resp.bytes_stream().map(|chunk| {
            chunk.map_err(|e| DenError::System(format!("LLM stream read failed: {e}")))
        }))
    }
}
