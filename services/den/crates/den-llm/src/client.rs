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

use den_core::{config::Config, DenError, ThinkingEffort};

#[cfg(test)]
use crate::model_registry;

/// Canonical Den model handle used in product/UI/DB/policy state, e.g. `openai/gpt-5.5`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenModelHandle(String);

impl DenModelHandle {
    pub fn normalize(model: &str) -> Self {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return Self("openai/gpt-4o-mini".to_string());
        }
        if trimmed.contains('/') {
            Self(trimmed.to_string())
        } else {
            Self(format!("openai/{trimmed}"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for DenModelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Provider/Bifrost request model id, e.g. `gpt-5.5` for Den handle `openai/gpt-5.5`.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModelId(String);

#[cfg(test)]
impl ProviderModelId {
    pub fn for_den_handle(handle: &DenModelHandle) -> Self {
        Self(
            model_registry::provider_model_id_for_handle(handle.as_str())
                .unwrap_or(handle.as_str())
                .to_string(),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
impl std::fmt::Display for ProviderModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bifrost expects `provider/model`; bare OpenAI-style ids get an `openai/` prefix.
pub fn normalize_llm_model_handle(model: &str) -> String {
    DenModelHandle::normalize(model).into_string()
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
    pub bifrost_virtual_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmApiStyle {
    ChatCompletionsStream,
    ResponsesStream,
}

impl LlmApiStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            LlmApiStyle::ChatCompletionsStream => "chat_completions_stream",
            LlmApiStyle::ResponsesStream => "responses_stream",
        }
    }
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
    pub thinking_effort: Option<ThinkingEffort>,
    pub telemetry: Option<LlmRequestTelemetry>,
}

fn log_sample(value: &str, max_chars: usize) -> String {
    let mut sample = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        sample.push('…');
    }
    sample
}

fn chat_message_chain_diagnostic(messages: &[ChatMessage]) -> Value {
    let mut assistant_tool_call_blocks = 0_u64;
    let mut tool_messages = 0_u64;
    let mut orphan_tool_messages = Vec::<String>::new();
    let mut unresolved_tool_call_ids = Vec::<String>::new();
    let mut duplicate_tool_result_ids = Vec::<String>::new();
    let mut role_tail = Vec::<String>::new();
    let mut index = 0_usize;

    while index < messages.len() {
        let message = &messages[index];
        if message.role == "assistant" {
            if let Some(calls) = message
                .tool_calls
                .as_ref()
                .filter(|calls| !calls.is_empty())
            {
                assistant_tool_call_blocks += 1;
                let required = calls.iter().map(|call| call.id.clone()).collect::<Vec<_>>();
                let mut seen = std::collections::HashSet::<String>::new();
                let mut scan = index + 1;
                while scan < messages.len() && messages[scan].role == "tool" {
                    tool_messages += 1;
                    let tool_call_id = messages[scan]
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| "<missing>".to_string());
                    if !required.iter().any(|id| id == &tool_call_id) {
                        orphan_tool_messages.push(tool_call_id);
                    } else if !seen.insert(tool_call_id.clone()) {
                        duplicate_tool_result_ids.push(tool_call_id);
                    }
                    scan += 1;
                }
                for required_id in required {
                    if !seen.contains(&required_id) {
                        unresolved_tool_call_ids.push(required_id);
                    }
                }
                index = scan;
                continue;
            }
        } else if message.role == "tool" {
            tool_messages += 1;
            orphan_tool_messages.push(
                message
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| "<missing>".to_string()),
            );
        }
        index += 1;
    }

    let tail_start = messages.len().saturating_sub(12);
    for message in &messages[tail_start..] {
        let role = if let Some(calls) = message
            .tool_calls
            .as_ref()
            .filter(|calls| !calls.is_empty())
        {
            format!("{}[tool_calls:{}]", message.role, calls.len())
        } else if message.role == "tool" {
            format!(
                "tool[{}]",
                message.tool_call_id.as_deref().unwrap_or("<missing>")
            )
        } else {
            message.role.clone()
        };
        role_tail.push(role);
    }

    let invalid = !orphan_tool_messages.is_empty()
        || !unresolved_tool_call_ids.is_empty()
        || !duplicate_tool_result_ids.is_empty();
    json!({
        "invalid": invalid,
        "message_count": messages.len(),
        "assistant_tool_call_blocks": assistant_tool_call_blocks,
        "tool_messages": tool_messages,
        "orphan_tool_messages": orphan_tool_messages,
        "unresolved_tool_call_ids": unresolved_tool_call_ids,
        "duplicate_tool_result_ids": duplicate_tool_result_ids,
        "role_tail": role_tail,
    })
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
            "bifrost_virtual_key" => self.bifrost_virtual_key.as_deref(),
            _ => None,
        }
    }
}

fn telemetry_field_or_unknown<'a>(
    telemetry: Option<&'a LlmRequestTelemetry>,
    name: &str,
) -> &'a str {
    telemetry
        .and_then(|telemetry| telemetry.field(name))
        .unwrap_or("unknown")
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
            // Bifrost requires provider-qualified ids (`provider/model`) on both
            // `/v1/completions` and `/v1/responses`; normalize defensively.
            "model": normalize_llm_model_handle(&self.model),
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
        if let Some(thinking_effort) = self.thinking_effort {
            body["reasoning_effort"] = json!(thinking_effort.as_str());
        }
        body
    }

    pub fn to_responses_body(&self) -> Value {
        let input: Vec<Value> = self
            .messages
            .iter()
            .flat_map(chat_message_to_responses_input_items)
            .collect();
        let tools: Vec<Value> = self
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                })
            })
            .collect();
        // Bifrost's `/v1/responses` requires provider-qualified ids (`provider/model`),
        // same as `/v1/completions`; sending a bare id yields HTTP 400
        // "model should be in provider/model format".
        let mut body = json!({
            "model": normalize_llm_model_handle(&self.model),
            "input": input,
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
            body["max_output_tokens"] = json!(max_tokens);
        }
        if let Some(thinking_effort) = self.thinking_effort {
            body["reasoning"] = json!({
                "effort": thinking_effort.as_str(),
            });
        }
        body
    }
}

fn chat_message_to_responses_input_items(message: &ChatMessage) -> Vec<Value> {
    let content = message.content.clone().unwrap_or_default();
    if message.role == "tool" {
        return vec![json!({
            "type": "function_call_output",
            "call_id": message.tool_call_id.clone().unwrap_or_default(),
            "output": content,
        })];
    }

    let mut items = Vec::new();
    if !content.is_empty() || message.tool_calls.is_none() {
        items.push(json!({
            "role": message.role,
            "content": content,
        }));
    }

    if let Some(tool_calls) = &message.tool_calls {
        for tool_call in tool_calls {
            items.push(json!({
                "type": "function_call",
                "call_id": &tool_call.id,
                "name": &tool_call.function.name,
                "arguments": &tool_call.function.arguments,
            }));
        }
    }

    items
}

pub fn preferred_api_style_for_model(model: &str) -> LlmApiStyle {
    preferred_api_style_for_model_with_catalog_support(model, None)
}

pub fn preferred_api_style_for_model_with_catalog_support(
    model: &str,
    supports_responses_api: Option<bool>,
) -> LlmApiStyle {
    if let Some(supports_responses_api) = supports_responses_api {
        return if supports_responses_api {
            LlmApiStyle::ResponsesStream
        } else {
            LlmApiStyle::ChatCompletionsStream
        };
    }
    let model = normalize_llm_model_handle(model);
    if model == "openai/gpt-5.5" || model.ends_with("/gpt-5.5") {
        LlmApiStyle::ResponsesStream
    } else {
        LlmApiStyle::ChatCompletionsStream
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

fn apply_bears_telemetry_headers(
    mut req: reqwest::RequestBuilder,
    telemetry: Option<&LlmRequestTelemetry>,
) -> reqwest::RequestBuilder {
    let Some(telemetry) = telemetry else {
        return req;
    };
    req = apply_optional_header(req, "x-bears-request-id", telemetry.field("request_id"));
    req = apply_optional_header(req, "x-bears-run-id", telemetry.field("run_id"));
    req = apply_optional_header(req, "x-bears-session-id", telemetry.field("session_id"));
    req = apply_optional_header(
        req,
        "x-bears-conversation-id",
        telemetry.field("conversation_id"),
    );
    req = apply_optional_header(req, "x-bears-bear-id", telemetry.field("bear_id"));
    apply_optional_header(req, "x-bears-stance", telemetry.field("stance"))
}

fn has_bifrost_virtual_key(telemetry: Option<&LlmRequestTelemetry>) -> bool {
    telemetry
        .and_then(|telemetry| telemetry.field("bifrost_virtual_key"))
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

fn require_bifrost_virtual_key(telemetry: Option<&LlmRequestTelemetry>) -> Result<(), DenError> {
    if has_bifrost_virtual_key(telemetry) {
        return Ok(());
    }
    let bear_id = telemetry
        .and_then(|telemetry| telemetry.bear_id.as_deref())
        .unwrap_or("unknown");
    let conversation_id = telemetry
        .and_then(|telemetry| telemetry.conversation_id.as_deref())
        .unwrap_or("unknown");
    Err(DenError::System(format!(
        "Bifrost virtual key is required for LLM inference, but none is configured for this request. Provision a Bear-scoped Bifrost virtual key before running this Bear. Context: bear_id={bear_id}, conversation_id={conversation_id}."
    )))
}

fn apply_bifrost_virtual_key_header(
    req: reqwest::RequestBuilder,
    telemetry: Option<&LlmRequestTelemetry>,
) -> reqwest::RequestBuilder {
    let Some(telemetry) = telemetry else {
        return req;
    };
    // Bifrost virtual keys are inference credentials. In Bifrost 1.6.x, use the
    // dedicated governance virtual-key header for inference; `x-api-key` can be
    // interpreted as a provider/API key path and may fail virtual-key lookup on
    // responses/chat requests even when management usage views work.
    apply_optional_header(req, "x-bf-vk", telemetry.field("bifrost_virtual_key"))
}

fn apply_bifrost_session_cache_headers(
    mut req: reqwest::RequestBuilder,
    telemetry: Option<&LlmRequestTelemetry>,
) -> reqwest::RequestBuilder {
    let Some(telemetry) = telemetry else {
        return req;
    };
    // Bifrost session stickiness: keep a conversation pinned to the same
    // upstream key via the documented x-bf-session-id header. Direct cache
    // uses the same partition key and is forced to hash-only mode; no
    // embeddings are required.
    req = apply_optional_header(req, "x-bf-session-id", telemetry.field("conversation_id"));
    req = apply_optional_header(req, "x-bf-cache-key", telemetry.field("conversation_id"));
    apply_optional_header(req, "x-bf-cache-type", Some("direct"))
}

pub fn bifrost_key_selection_error(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("no keys found that support model")
        || (text.contains("key-selection error") && text.contains("no keys"))
}

fn bifrost_virtual_key_not_found_error(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("virtual_key_not_found") || text.contains("virtual key not found")
}

fn bifrost_virtual_key_not_found_diagnostic(
    api_style: LlmApiStyle,
    model: &str,
    telemetry: Option<&LlmRequestTelemetry>,
    status: &str,
    text: &str,
) -> String {
    let request_id = telemetry_field_or_unknown(telemetry, "request_id");
    let session_id = telemetry_field_or_unknown(telemetry, "session_id");
    let conversation_id = telemetry_field_or_unknown(telemetry, "conversation_id");
    let bear_id = telemetry_field_or_unknown(telemetry, "bear_id");
    let virtual_key_present = telemetry
        .and_then(|t| t.field("bifrost_virtual_key"))
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let api_style = api_style.as_str();
    format!(
        "Bifrost virtual-key lookup failed for {api_style} model {model}. Bifrost reported HTTP {status}: {text}. Meaning: Den sent a Bifrost virtual key as x-bf-vk (present={virtual_key_present}), but Bifrost could not find that virtual key in its runtime governance config store. Check that the Bear's stored encrypted virtual-key secret still exists, that the corresponding key exists in Bifrost /api/governance/virtual-keys, that Bifrost config_store is persistent, and that /app/data/config.db was not reset after Den stored the Bear mapping. Context: request_id={request_id}, session_id={session_id}, conversation_id={conversation_id}, bear_id={bear_id}."
    )
}

fn bifrost_key_selection_diagnostic(
    api_style: LlmApiStyle,
    model: &str,
    telemetry: Option<&LlmRequestTelemetry>,
    status: &str,
    text: &str,
    retry_status: Option<&str>,
    retry_text: Option<&str>,
) -> String {
    let request_id = telemetry_field_or_unknown(telemetry, "request_id");
    let session_id = telemetry_field_or_unknown(telemetry, "session_id");
    let conversation_id = telemetry_field_or_unknown(telemetry, "conversation_id");
    let retry_summary = match (retry_status, retry_text) {
        (Some(retry_status), Some(retry_text)) => format!(
            " Den retried once without x-bf-session-id/x-bf-cache-* headers and Bifrost still returned HTTP {retry_status}: {retry_text}."
        ),
        _ => String::new(),
    };
    let api_style = api_style.as_str();
    format!(
        "Bifrost key-selection failed for {api_style} model {model}. Bifrost reported HTTP {status}: {text}.{retry_summary} Meaning: after Bifrost applied provider/model routing, session/key stickiness, request-type filters, and key model allowlists, no eligible provider key remained. If this is stochastic, inspect Bifrost for mixed OpenAI keys, stale config-store key rows, per-key `models` allowlists, key health/rate-limit state, and Responses-vs-chat support for this model. If it only affects old sessions, clear Bifrost session/cache state or start a new Den/ACP session. Context: request_id={request_id}, session_id={session_id}, conversation_id={conversation_id}."
    )
}

struct TimedLlmByteStream {
    inner: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    telemetry: Option<LlmRequestTelemetry>,
    model: String,
    api_style: LlmApiStyle,
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
        api_style: LlmApiStyle,
        request_started_at: Instant,
        headers_received_at: Instant,
    ) -> Self {
        Self {
            inner: Box::pin(response.bytes_stream()),
            telemetry,
            model,
            api_style,
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
            api_style = %self.api_style.as_str(),
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
            "LLM stream timing summary"
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
                        api_style = %self.api_style.as_str(),
                        request_id = telemetry.and_then(|t| t.request_id.as_deref()),
                        run_id = telemetry.and_then(|t| t.run_id.as_deref()),
                        session_id = telemetry.and_then(|t| t.session_id.as_deref()),
                        conversation_id = telemetry.and_then(|t| t.conversation_id.as_deref()),
                        headers_to_first_byte_ms = now.duration_since(self.headers_received_at).as_millis(),
                        request_to_first_byte_ms = now.duration_since(self.request_started_at).as_millis(),
                        "LLM first response byte received"
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
    default_model: String,
}

impl LlmClient {
    pub fn new(config: &Config) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_mins(5))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client for llm");
        Self {
            http,
            base_url: config.llm_api_url.trim_end_matches('/').to_string(),
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
        require_bifrost_virtual_key(request.telemetry.as_ref())?;
        let url = format!("{}/chat/completions", self.base_url);
        let message_chain_diagnostic = chat_message_chain_diagnostic(&request.messages);
        if message_chain_diagnostic
            .get("invalid")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            tracing::warn!(
                model = %request.model,
                request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                run_id = request.telemetry.as_ref().and_then(|t| t.run_id.as_deref()),
                session_id = request.telemetry.as_ref().and_then(|t| t.session_id.as_deref()),
                conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
                diagnostic = %message_chain_diagnostic,
                "LLM chat/completions request preflight rejected invalid tool-call message chain"
            );
            return Err(DenError::System(format!(
                "LLM chat/completions request preflight rejected invalid tool-call message chain: {message_chain_diagnostic}"
            )));
        }
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
            message_chain_diagnostic = %message_chain_diagnostic,
            "LLM chat/completions request starting"
        );
        let started = Instant::now();
        let body = request.to_body();
        let mut req = self.http.post(&url).json(&body);
        req = apply_bears_telemetry_headers(req, request.telemetry.as_ref());
        req = apply_bifrost_virtual_key_header(req, request.telemetry.as_ref());
        req = apply_bifrost_session_cache_headers(req, request.telemetry.as_ref());
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
            let response_body_sample = log_sample(&text, 1000);
            tracing::warn!(
                model = %request.model,
                http_status,
                duration_ms = started.elapsed().as_millis(),
                response_body_len = text.len(),
                response_body_sample = %response_body_sample,
                request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                run_id = request.telemetry.as_ref().and_then(|t| t.run_id.as_deref()),
                session_id = request.telemetry.as_ref().and_then(|t| t.session_id.as_deref()),
                conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
                "LLM chat/completions returned error status"
            );
            if bifrost_virtual_key_not_found_error(&text) {
                return Err(DenError::System(bifrost_virtual_key_not_found_diagnostic(
                    LlmApiStyle::ChatCompletionsStream,
                    &request.model,
                    request.telemetry.as_ref(),
                    &status.to_string(),
                    &text,
                )));
            }
            if bifrost_key_selection_error(&text) {
                tracing::warn!(
                    model = %request.model,
                    request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                    conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
                    "retrying LLM chat/completions once without Bifrost session/cache headers after key-selection error"
                );
                let mut retry_req = self.http.post(&url).json(&body);
                retry_req = apply_bears_telemetry_headers(retry_req, request.telemetry.as_ref());
                retry_req = apply_bifrost_virtual_key_header(retry_req, request.telemetry.as_ref());
                let retry_resp = retry_req.send().await.map_err(|e| {
                    let diagnostic = bifrost_key_selection_diagnostic(
                        LlmApiStyle::ChatCompletionsStream,
                        &request.model,
                        request.telemetry.as_ref(),
                        &status.to_string(),
                        &text,
                        None,
                        None,
                    );
                    DenError::System(format!(
                        "LLM chat/completions retry without Bifrost session/cache headers failed: {e}. {diagnostic}"
                    ))
                })?;
                if retry_resp.status().is_success() {
                    tracing::info!(
                        model = %request.model,
                        http_status = retry_resp.status().as_u16(),
                        request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                        conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
                        "LLM chat/completions retry without Bifrost session/cache headers succeeded"
                    );
                    return Ok(retry_resp);
                }
                let retry_status = retry_resp.status();
                let retry_text = retry_resp.text().await.unwrap_or_default();
                let diagnostic = bifrost_key_selection_diagnostic(
                    LlmApiStyle::ChatCompletionsStream,
                    &request.model,
                    request.telemetry.as_ref(),
                    &status.to_string(),
                    &text,
                    Some(&retry_status.to_string()),
                    Some(&retry_text),
                );
                tracing::warn!(
                    model = %request.model,
                    request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                    conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
                    diagnostic = %diagnostic,
                    "LLM chat/completions key-selection retry diagnostic"
                );
                return Err(DenError::System(diagnostic));
            }
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
            "LLM chat/completions response headers received"
        );
        Ok(resp)
    }

    pub async fn responses_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<Response, DenError> {
        if !self.is_enabled() {
            return Err(DenError::System(
                "LLM API is not configured (set LLM_API_URL or BIFROST_BASE_URL)".to_string(),
            ));
        }
        require_bifrost_virtual_key(request.telemetry.as_ref())?;
        let url = format!("{}/responses", self.base_url);
        let model_handle = DenModelHandle::normalize(&request.model);
        tracing::info!(
            model_handle = %model_handle,
            api_style = %LlmApiStyle::ResponsesStream.as_str(),
            message_count = request.messages.len(),
            tool_count = request.tools.len(),
            stream = request.stream,
            llm_url = %url,
            request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
            run_id = request.telemetry.as_ref().and_then(|t| t.run_id.as_deref()),
            session_id = request.telemetry.as_ref().and_then(|t| t.session_id.as_deref()),
            conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
            "LLM responses request starting"
        );
        let started = Instant::now();
        let body = request.to_responses_body();
        let mut req = self.http.post(&url).json(&body);
        req = apply_bears_telemetry_headers(req, request.telemetry.as_ref());
        req = apply_bifrost_virtual_key_header(req, request.telemetry.as_ref());
        req = apply_bifrost_session_cache_headers(req, request.telemetry.as_ref());
        let resp = req.send().await.map_err(|e| {
            tracing::warn!(
                model_handle = %model_handle,
                api_style = %LlmApiStyle::ResponsesStream.as_str(),
                duration_ms = started.elapsed().as_millis(),
                error = %e,
                "LLM responses request failed"
            );
            DenError::System(format!("LLM responses request failed: {e}"))
        })?;
        let http_status = resp.status().as_u16();
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let response_body_sample = log_sample(&text, 1000);
            tracing::warn!(
                model_handle = %model_handle,
                api_style = %LlmApiStyle::ResponsesStream.as_str(),
                http_status,
                duration_ms = started.elapsed().as_millis(),
                response_body_len = text.len(),
                response_body_sample = %response_body_sample,
                request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                run_id = request.telemetry.as_ref().and_then(|t| t.run_id.as_deref()),
                session_id = request.telemetry.as_ref().and_then(|t| t.session_id.as_deref()),
                conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
                "LLM responses returned error status"
            );
            if bifrost_virtual_key_not_found_error(&text) {
                return Err(DenError::System(bifrost_virtual_key_not_found_diagnostic(
                    LlmApiStyle::ResponsesStream,
                    model_handle.as_str(),
                    request.telemetry.as_ref(),
                    &status.to_string(),
                    &text,
                )));
            }
            if bifrost_key_selection_error(&text) {
                tracing::warn!(
                    model_handle = %model_handle,
                    api_style = %LlmApiStyle::ResponsesStream.as_str(),
                    request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                    conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
                    "retrying LLM responses once without Bifrost session/cache headers after key-selection error"
                );
                let mut retry_req = self.http.post(&url).json(&body);
                retry_req = apply_bears_telemetry_headers(retry_req, request.telemetry.as_ref());
                retry_req = apply_bifrost_virtual_key_header(retry_req, request.telemetry.as_ref());
                let retry_resp = retry_req.send().await.map_err(|e| {
                    let diagnostic = bifrost_key_selection_diagnostic(
                        LlmApiStyle::ResponsesStream,
                        model_handle.as_str(),
                        request.telemetry.as_ref(),
                        &status.to_string(),
                        &text,
                        None,
                        None,
                    );
                    DenError::System(format!(
                        "LLM responses retry without Bifrost session/cache headers failed: {e}. {diagnostic}"
                    ))
                })?;
                if retry_resp.status().is_success() {
                    tracing::info!(
                        model_handle = %model_handle,
                        api_style = %LlmApiStyle::ResponsesStream.as_str(),
                        http_status = retry_resp.status().as_u16(),
                        request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                        conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
                        "LLM responses retry without Bifrost session/cache headers succeeded"
                    );
                    return Ok(retry_resp);
                }
                let retry_status = retry_resp.status();
                let retry_text = retry_resp.text().await.unwrap_or_default();
                let diagnostic = bifrost_key_selection_diagnostic(
                    LlmApiStyle::ResponsesStream,
                    model_handle.as_str(),
                    request.telemetry.as_ref(),
                    &status.to_string(),
                    &text,
                    Some(&retry_status.to_string()),
                    Some(&retry_text),
                );
                tracing::warn!(
                    model_handle = %model_handle,
                    api_style = %LlmApiStyle::ResponsesStream.as_str(),
                    request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
                    conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
                    diagnostic = %diagnostic,
                    "LLM responses key-selection retry diagnostic"
                );
                return Err(DenError::System(diagnostic));
            }
            return Err(DenError::System(format!(
                "LLM responses HTTP {status}: {text}"
            )));
        }
        tracing::info!(
            model_handle = %model_handle,
            api_style = %LlmApiStyle::ResponsesStream.as_str(),
            http_status,
            duration_ms = started.elapsed().as_millis(),
            request_id = request.telemetry.as_ref().and_then(|t| t.request_id.as_deref()),
            run_id = request.telemetry.as_ref().and_then(|t| t.run_id.as_deref()),
            session_id = request.telemetry.as_ref().and_then(|t| t.session_id.as_deref()),
            conversation_id = request.telemetry.as_ref().and_then(|t| t.conversation_id.as_deref()),
            "LLM responses headers received"
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
            LlmApiStyle::ChatCompletionsStream,
            started,
            headers_received_at,
        ))
    }

    pub async fn responses_byte_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<impl Stream<Item = Result<bytes::Bytes, DenError>> + Send + Unpin, DenError> {
        let started = Instant::now();
        let resp = self.responses_stream(request).await?;
        let headers_received_at = Instant::now();
        Ok(TimedLlmByteStream::new(
            resp,
            request.telemetry.clone(),
            request.model.clone(),
            LlmApiStyle::ResponsesStream,
            started,
            headers_received_at,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_call(id: &str) -> ChatToolCall {
        ChatToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: ChatToolCallFunction {
                name: "memory_read".to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    #[test]
    fn model_identity_wrappers_separate_den_handle_from_provider_id() {
        let handle = DenModelHandle::normalize("gpt-5.5");
        let provider = ProviderModelId::for_den_handle(&handle);

        assert_eq!(handle.as_str(), "openai/gpt-5.5");
        assert_eq!(provider.as_str(), "gpt-5.5");
    }

    #[test]
    fn preferred_api_style_uses_explicit_catalog_support() {
        assert_eq!(
            preferred_api_style_for_model_with_catalog_support("vendor/style-yes", Some(true)),
            LlmApiStyle::ResponsesStream
        );
        assert_eq!(
            preferred_api_style_for_model_with_catalog_support("vendor/style-no", Some(false)),
            LlmApiStyle::ChatCompletionsStream
        );
    }

    #[test]
    fn preferred_api_style_falls_back_to_literal_before_catalog() {
        // A handle the catalog has not advertised falls back to the static rule.
        assert_eq!(
            preferred_api_style_for_model("vendor/never-observed-5.5-like"),
            LlmApiStyle::ChatCompletionsStream
        );
        assert_eq!(
            preferred_api_style_for_model("gpt-5.5"),
            LlmApiStyle::ResponsesStream
        );
    }

    #[test]
    fn detects_bifrost_key_selection_errors() {
        let raw = r#"LLM responses HTTP 400 Bad Request: {"is_bifrost_error":false,"error":{"error":"no keys found that support model: gpt-5.5","message":"no keys found that support model: gpt-5.5"},"extra_fields":{"provider":"openai","model_requested":"gpt-5.5","request_type":"responses_stream"}}"#;
        assert!(bifrost_key_selection_error(raw));

        let wrapped = format!(
            "{raw} (retry without Bifrost session/cache headers after key-selection error also failed; original HTTP 400 Bad Request: {raw})"
        );
        assert!(bifrost_key_selection_error(&wrapped));
    }

    #[test]
    fn responses_body_sends_provider_qualified_handle() {
        let request = ChatCompletionRequest {
            model: "openai/gpt-5.5".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some("ping".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            tools: Vec::new(),
            stream: true,
            tool_choice: None,
            temperature: None,
            max_tokens: None,
            thinking_effort: None,
            telemetry: None,
        };

        let body = request.to_responses_body();

        // Bifrost `/v1/responses` requires the provider-qualified handle, not a bare id.
        assert_eq!(body["model"], "openai/gpt-5.5");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn thinking_effort_serializes_for_chat_and_responses_requests() {
        let request = ChatCompletionRequest {
            model: "openai/gpt-5.5".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some("ping".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            }],
            tools: Vec::new(),
            stream: true,
            tool_choice: None,
            temperature: None,
            max_tokens: None,
            thinking_effort: Some(ThinkingEffort::High),
            telemetry: None,
        };

        let chat_body = request.to_body();
        assert_eq!(chat_body["reasoning_effort"], "high");

        let responses_body = request.to_responses_body();
        assert_eq!(responses_body["reasoning"]["effort"], "high");
    }

    #[test]
    fn chat_message_chain_diagnostic_accepts_complete_tool_chain() {
        let diagnostic = chat_message_chain_diagnostic(&[
            ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![tool_call("call_1")]),
            },
            ChatMessage {
                role: "tool".to_string(),
                content: Some("{}".to_string()),
                tool_call_id: Some("call_1".to_string()),
                name: Some("memory_read".to_string()),
                tool_calls: None,
            },
        ]);

        assert_eq!(diagnostic["invalid"], false);
        assert_eq!(diagnostic["assistant_tool_call_blocks"], 1);
        assert_eq!(diagnostic["tool_messages"], 1);
    }

    #[test]
    fn chat_message_chain_diagnostic_rejects_unresolved_tool_call() {
        let diagnostic = chat_message_chain_diagnostic(&[ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_call_id: None,
            name: None,
            tool_calls: Some(vec![tool_call("call_1")]),
        }]);

        assert_eq!(diagnostic["invalid"], true);
        assert_eq!(diagnostic["unresolved_tool_call_ids"][0], "call_1");
    }

    #[test]
    fn chat_message_chain_diagnostic_rejects_orphan_tool_message() {
        let diagnostic = chat_message_chain_diagnostic(&[ChatMessage {
            role: "tool".to_string(),
            content: Some("{}".to_string()),
            tool_call_id: Some("call_orphan".to_string()),
            name: Some("memory_read".to_string()),
            tool_calls: None,
        }]);

        assert_eq!(diagnostic["invalid"], true);
        assert_eq!(diagnostic["orphan_tool_messages"][0], "call_orphan");
    }
}
