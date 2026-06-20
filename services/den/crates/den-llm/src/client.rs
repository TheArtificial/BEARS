use std::{
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use futures::Stream;
use reqwest::{
    header::{HeaderName, HeaderValue},
    Response,
};
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

#[derive(Debug, Clone, Default)]
pub struct LlmRequestTelemetry {
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub conversation_id: Option<String>,
    pub bear_id: Option<String>,
    pub stance: Option<String>,
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
    pub telemetry: Option<LlmRequestTelemetry>,
}

impl LlmRequestTelemetry {
    fn field(&self, name: &str) -> Option<&str> {
        match name {
            "request_id" => self.request_id.as_deref(),
            "run_id" => self.run_id.as_deref(),
            "session_id" => self.session_id.as_deref(),
            "conversation_id" => self.conversation_id.as_deref(),
            "bear_id" => self.bear_id.as_deref(),
            "stance" => self.stance.as_deref(),
            _ => None,
        }
    }
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

fn apply_optional_header(
    req: reqwest::RequestBuilder,
    name: &'static str,
    value: Option<&str>,
) -> reqwest::RequestBuilder {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return req;
    };
    let Ok(value) = HeaderValue::from_str(value) else {
        tracing::warn!(header = name, "skipping invalid LLM telemetry header value");
        return req;
    };
    req.header(HeaderName::from_static(name), value)
}

struct TimedLlmByteStream {
    inner: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    telemetry: Option<LlmRequestTelemetry>,
    model: String,
    request_started_at: Instant,
    headers_received_at: Instant,
    first_byte_received_at: Option<Instant>,
    chunk_count: u64,
    byte_count: u64,
    completed: bool,
}

impl TimedLlmByteStream {
    fn new(
        response: Response,
        telemetry: Option<LlmRequestTelemetry>,
        model: String,
        request_started_at: Instant,
        headers_received_at: Instant,
    ) -> Self {
        Self {
            inner: Box::pin(response.bytes_stream()),
            telemetry,
            model,
            request_started_at,
            headers_received_at,
            first_byte_received_at: None,
            chunk_count: 0,
            byte_count: 0,
            completed: false,
        }
    }

    fn log_summary(&self, status: &str, error: Option<&str>) {
        let telemetry = self.telemetry.as_ref();
        tracing::info!(
            status,
            error,
            model = %self.model,
            request_id = telemetry.and_then(|t| t.request_id.as_deref()),
            run_id = telemetry.and_then(|t| t.run_id.as_deref()),
            session_id = telemetry.and_then(|t| t.session_id.as_deref()),
            conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
            bear_id = telemetry.and_then(|t| t.bear_id.as_deref()),
            stance = telemetry.and_then(|t| t.stance.as_deref()),
            request_to_headers_ms = self.headers_received_at.duration_since(self.request_started_at).as_millis(),
            headers_to_first_byte_ms = self.first_byte_received_at.map(|at| at.duration_since(self.headers_received_at).as_millis()),
            request_to_first_byte_ms = self.first_byte_received_at.map(|at| at.duration_since(self.request_started_at).as_millis()),
            total_ms = self.headers_received_at.elapsed().as_millis() + self.headers_received_at.duration_since(self.request_started_at).as_millis(),
            chunk_count = self.chunk_count,
            byte_count = self.byte_count,
            "LLM chat/completions stream timing summary"
        );
    }
}

impl Unpin for TimedLlmByteStream {}

impl Stream for TimedLlmByteStream {
    type Item = Result<bytes::Bytes, DenError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                if self.first_byte_received_at.is_none() {
                    let now = Instant::now();
                    self.first_byte_received_at = Some(now);
                    let telemetry = self.telemetry.as_ref();
                    tracing::info!(
                        model = %self.model,
                        request_id = telemetry.and_then(|t| t.request_id.as_deref()),
                        run_id = telemetry.and_then(|t| t.run_id.as_deref()),
                        session_id = telemetry.and_then(|t| t.session_id.as_deref()),
                        conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
                        headers_to_first_byte_ms = now.duration_since(self.headers_received_at).as_millis(),
                        request_to_first_byte_ms = now.duration_since(self.request_started_at).as_millis(),
                        "LLM chat/completions first response byte received"
                    );
                }
                self.chunk_count = self.chunk_count.saturating_add(1);
                self.byte_count = self.byte_count.saturating_add(bytes.len() as u64);
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(err))) => {
                self.completed = true;
                let message = err.to_string();
                self.log_summary("error", Some(&message));
                Poll::Ready(Some(Err(DenError::System(format!(
                    "LLM stream read failed: {err}"
                )))))
            }
            Poll::Ready(None) => {
                self.completed = true;
                self.log_summary("completed", None);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for TimedLlmByteStream {
    fn drop(&mut self) {
        if !self.completed {
            self.log_summary("dropped", None);
        }
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
            request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
            run_id = request.telemetry.as_ref().and_then(|t| t.run_id.as_deref()),
            session_id = request.telemetry.as_ref().and_then(|t| t.session_id.as_deref()),
            conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
            bear_id = request.telemetry.as_ref().and_then(|t| t.bear_id.as_deref()),
            stance = request.telemetry.as_ref().and_then(|t| t.stance.as_deref()),
            x_model_affinity = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
            "LLM chat/completions request starting"
        );
        let started = Instant::now();
        let mut req = self.http.post(&url).json(&request.to_body());
        if let Some(telemetry) = request.telemetry.as_ref() {
            req = apply_optional_header(req, "x-bears-request-id", telemetry.field("request_id"));
            req = apply_optional_header(req, "x-bears-run-id", telemetry.field("run_id"));
            req = apply_optional_header(req, "x-bears-session-id", telemetry.field("session_id"));
            req = apply_optional_header(
                req,
                "x-bears-conversation-id",
                telemetry.field("conversation_id"),
            );
            req = apply_optional_header(req, "x-bears-bear-id", telemetry.field("bear_id"));
            req = apply_optional_header(req, "x-bears-stance", telemetry.field("stance"));
            req =
                apply_optional_header(req, "x-model-affinity", telemetry.field("conversation_id"));
        }
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req.send().await.map_err(|e| {
            tracing::warn!(
                model = %request.model,
                duration_ms = started.elapsed().as_millis(),
                error = %e,
                request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                run_id = request.telemetry.as_ref().and_then(|t| t.run_id.as_deref()),
                session_id = request.telemetry.as_ref().and_then(|t| t.session_id.as_deref()),
                conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
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
                request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                run_id = request.telemetry.as_ref().and_then(|t| t.run_id.as_deref()),
                session_id = request.telemetry.as_ref().and_then(|t| t.session_id.as_deref()),
                conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
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
            request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
            run_id = request.telemetry.as_ref().and_then(|t| t.run_id.as_deref()),
            session_id = request.telemetry.as_ref().and_then(|t| t.session_id.as_deref()),
            conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
            x_model_affinity = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
            "LLM chat/completions response headers received"
        );
        Ok(resp)
    }

    pub async fn chat_completions_byte_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<impl Stream<Item = Result<bytes::Bytes, DenError>> + Send + Unpin, DenError> {
        let started = Instant::now();
        let resp = self.chat_completions_stream(request).await?;
        let headers_received_at = Instant::now();
        Ok(TimedLlmByteStream::new(
            resp,
            request.telemetry.clone(),
            request.model.clone(),
            started,
            headers_received_at,
        ))
    }
}
