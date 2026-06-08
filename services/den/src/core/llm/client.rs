use std::time::Duration;

use futures::{Stream, StreamExt};
use reqwest::Response;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{config::Config, errors::CustomError};

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
            default_model: config.default_llm_model.clone(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.base_url.is_empty()
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    pub fn resolve_model<'a>(&'a self, requested: Option<&'a str>) -> &'a str {
        requested
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or(&self.default_model)
    }

    pub async fn chat_completions_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<Response, CustomError> {
        if !self.is_enabled() {
            return Err(CustomError::System(
                "LLM API is not configured (set LLM_API_URL or BIFROST_BASE_URL)".to_string(),
            ));
        }
        let url = format!("{}/chat/completions", self.base_url);
        let mut req = self.http.post(url).json(&request.to_body());
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| CustomError::System(format!("LLM chat/completions request failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CustomError::System(format!(
                "LLM chat/completions HTTP {status}: {text}"
            )));
        }
        Ok(resp)
    }

    pub async fn chat_completions_byte_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<impl Stream<Item = Result<bytes::Bytes, CustomError>> + Send + Unpin, CustomError>
    {
        let resp = self.chat_completions_stream(request).await?;
        Ok(resp.bytes_stream().map(|chunk| {
            chunk.map_err(|e| CustomError::System(format!("LLM stream read failed: {e}")))
        }))
    }
}
