use std::collections::BTreeMap;

use bytes::Bytes;
use tokio::sync::oneshot;
use uuid::Uuid;

use den_service::tool_turns::ToolResultRequest;

use den_core::client_tools::{
    client_tool_display_for_provider, client_tool_policy_json_for_provider, diag_phase,
    supported_provider_tool_names, ClientToolName,
};
use den_core::tools::descriptor::{
    builtin_den_tool_descriptor_for_provider_name, builtin_den_tool_descriptors,
    den_tool_display_json_for_provider, den_tool_policy_json_for_provider,
};
use den_docket::{TaskListItemStatus, TaskListLocalProjection};

#[derive(Debug)]
pub enum GatewayEvent {
    AssistantTextDelta {
        text: String,
    },
    ReasoningTextDelta {
        text: String,
    },
    StatusText {
        text: String,
    },
    TurnComplete {
        outcome: String,
    },
    TurnResult {
        status: String,
        reason: String,
        request_id: Option<String>,
        session_id: Option<String>,
        retryable: bool,
        diagnostics: serde_json::Value,
    },
    Error {
        message: String,
        detail: Option<String>,
        error_type: Option<String>,
        request_id: Option<String>,
        context: Option<serde_json::Value>,
    },
    ToolRequest {
        request_id: String,
        turn_id: String,
        tool_call_id: String,
        approval_request_id: Option<String>,
        tool_name: String,
        title: String,
        kind: String,
        args: serde_json::Value,
        approval_required: bool,
        approval_reason: Option<String>,
        result_tx: Option<oneshot::Sender<ToolResultRequest>>,
        result_rx: Option<oneshot::Receiver<ToolResultRequest>>,
    },
    PermissionRequest {
        request_id: String,
        permission_id: String,
        tool_call_id: String,
        tool_name: String,
        title: String,
        reason: String,
        target: serde_json::Value,
        options: Vec<String>,
    },
    PlanUpdate(TaskListLocalProjection),
    PlanUpdateJson {
        entries: Vec<serde_json::Value>,
    },
    PlanApprovalFallback {
        plan_id: Uuid,
        title: String,
        body: String,
        artifact_path: String,
        state: String,
        approval_status: String,
    },
    ModeUpdate {
        mode: String,
    },
    ConversationResolved {
        conversation_id: String,
    },
    SessionInfoUpdate {
        title: Option<String>,
        updated_at: Option<String>,
        meta: Option<serde_json::Value>,
    },
}

impl GatewayEvent {
    pub fn adapter_type(&self) -> &'static str {
        match self {
            Self::AssistantTextDelta { .. } => "assistant_text_delta",
            Self::ReasoningTextDelta { .. } => "reasoning_text_delta",
            Self::StatusText { .. } => "status_text",
            Self::TurnComplete { .. } => "turn_complete",
            Self::TurnResult { .. } => "turn_result",
            Self::Error { .. } => "error",
            Self::ToolRequest { .. } => "tool_request",
            Self::PermissionRequest { .. } => "permission_request",
            Self::PlanUpdate { .. }
            | Self::PlanUpdateJson { .. }
            | Self::PlanApprovalFallback { .. } => "plan_update",
            Self::ModeUpdate { .. } => "mode_update",
            Self::SessionInfoUpdate { .. } => "session_info_update",
            Self::ConversationResolved { .. } => "conversation_resolved",
        }
    }

    pub fn has_visible_output(&self) -> bool {
        match self {
            Self::AssistantTextDelta { text } | Self::StatusText { text } => !text.is_empty(),
            Self::ReasoningTextDelta { .. } => false,
            Self::Error { .. } => true,
            Self::TurnComplete { .. }
            | Self::TurnResult { .. }
            | Self::ToolRequest { .. }
            | Self::PermissionRequest { .. }
            | Self::PlanApprovalFallback { .. } => true,
            Self::PlanUpdate { .. }
            | Self::PlanUpdateJson { .. }
            | Self::ModeUpdate { .. }
            | Self::ConversationResolved { .. }
            | Self::SessionInfoUpdate { .. } => false,
        }
    }
}

pub fn provider_inner(msg: &serde_json::Value) -> &serde_json::Value {
    match msg.get("contents") {
        Some(c) if c.get("message_type").is_some() => c,
        _ => msg,
    }
}

pub fn provider_stream_text_preserving_whitespace(inner: &serde_json::Value) -> Option<String> {
    let content = inner.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(obj) = content.as_object() {
        if let Some(t) = obj.get("text").and_then(|x| x.as_str()) {
            return Some(t.to_string());
        }
    }
    let parts = content.as_array()?;
    let mut out = String::new();
    let mut found_text = false;
    for part in parts {
        if let Some(t) = part.get("text").and_then(|x| x.as_str()) {
            found_text = true;
            out.push_str(t);
        }
    }
    found_text.then_some(out)
}

pub fn map_provider_stream_event_to_gateway_event(
    event: &serde_json::Value,
) -> Option<GatewayEvent> {
    let inner = provider_inner(event);
    let message_type = inner
        .get("message_type")
        .and_then(|v| v.as_str())
        .or_else(|| event.get("message_type").and_then(|v| v.as_str()))
        .unwrap_or("");
    match message_type {
        "ping" => None,
        "assistant_message" => {
            let text = provider_stream_text_preserving_whitespace(inner)
                .or_else(|| provider_stream_text_preserving_whitespace(event))
                .unwrap_or_default();
            if let Some(tool_name) = pseudo_tool_call_name(&text) {
                Some(GatewayEvent::Error {
                    message: format!(
                        "Model emitted textual pseudo tool call for {tool_name} instead of a native tool call."
                    ),
                    detail: Some("The tool was advertised, but the model emitted text instead of a native tool call. This can happen when the continuation tool surface is too large, tool schema handling drifted inside model provider, or the run hit a continuation budget. Check `Posting provider armature tool return continuation` for client_tools_count/client_tools_bytes/max_steps.".to_string()),
                    error_type: Some("pseudo_tool_call_text".to_string()),
                    request_id: None,
                    context: Some(serde_json::json!({
                        "tool_name": tool_name,
                        "preview": preview_str_truncated(&text, 500),
                    })),
                })
            } else {
                Some(GatewayEvent::AssistantTextDelta { text })
            }
        }
        "reasoning_message" => Some(GatewayEvent::ReasoningTextDelta {
            text: inner
                .get("reasoning")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| {
                    event
                        .get("reasoning")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .or_else(|| provider_stream_text_preserving_whitespace(inner))
                .or_else(|| provider_stream_text_preserving_whitespace(event))
                .unwrap_or_default(),
        }),
        "error_message" => Some(GatewayEvent::Error {
            message: event
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Upstream error")
                .to_string(),
            detail: event
                .get("detail")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            error_type: event
                .get("error_type")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            request_id: event
                .get("request_id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            context: event.get("context").cloned(),
        }),
        "stop_reason" => {
            let stop_reason = inner
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .or_else(|| event.get("stop_reason").and_then(|v| v.as_str()))
                .unwrap_or("unknown");
            if stop_reason == "end_turn" {
                Some(GatewayEvent::TurnComplete {
                    outcome: "ok".to_string(),
                })
            } else if stop_reason == "requires_approval" {
                None
            } else {
                Some(GatewayEvent::Error {
                    message: format!(
                        "Runtime stopped before producing assistant output: {stop_reason}"
                    ),
                    detail: None,
                    error_type: Some(stop_reason.to_string()),
                    request_id: None,
                    context: None,
                })
            }
        }
        "tool_call_message" | "approval_request_message" | "function_call" => {
            native_provider_tool_request_event(
                event,
                inner,
                message_type == "approval_request_message",
            )
        }
        "tool_return_message" => None,
        _ => {
            conversation_resolved_gateway_event(event).or_else(|| extract_stream_text_delta(event))
        }
    }
}

fn extract_stream_text_delta(event: &serde_json::Value) -> Option<GatewayEvent> {
    let kind = stream_text_delta_kind(event);
    let reasoning_text = stream_reasoning_delta_text(event);
    let assistant_text = stream_assistant_delta_text(event);
    let (kind, text) = match kind {
        Some(StreamTextDeltaKind::Assistant) => (
            // ponytail: providers sometimes label every stream item as a message_delta;
            // an explicit reasoning field is still reasoning unless there is separate
            // assistant content in the same event. If providers start sending both in one
            // item, split the event upstream instead of guessing here.
            if reasoning_text.is_some() && assistant_text.is_none() {
                StreamTextDeltaKind::Reasoning
            } else {
                StreamTextDeltaKind::Assistant
            },
            assistant_text.or_else(|| reasoning_text.clone())?,
        ),
        Some(StreamTextDeltaKind::Reasoning) => (
            StreamTextDeltaKind::Reasoning,
            reasoning_text.or_else(|| assistant_text.clone())?,
        ),
        None => {
            if let Some(text) = reasoning_text {
                (StreamTextDeltaKind::Reasoning, text)
            } else if let Some(text) = assistant_text {
                (StreamTextDeltaKind::Assistant, text)
            } else {
                return None;
            }
        }
    };
    if text.is_empty() {
        return None;
    }
    match kind {
        StreamTextDeltaKind::Assistant => Some(GatewayEvent::AssistantTextDelta { text }),
        StreamTextDeltaKind::Reasoning => Some(GatewayEvent::ReasoningTextDelta { text }),
    }
}

#[derive(Clone, Copy)]
enum StreamTextDeltaKind {
    Assistant,
    Reasoning,
}

fn stream_text_delta_kind(event: &serde_json::Value) -> Option<StreamTextDeltaKind> {
    let candidates = [
        event.get("kind").and_then(|v| v.as_str()),
        event.get("role").and_then(|v| v.as_str()),
        event.get("type").and_then(|v| v.as_str()),
        event.get("message_type").and_then(|v| v.as_str()),
        event.pointer("/delta/kind").and_then(|v| v.as_str()),
        event.pointer("/delta/role").and_then(|v| v.as_str()),
        event
            .pointer("/choices/0/delta/role")
            .and_then(|v| v.as_str()),
    ];
    for candidate in candidates.into_iter().flatten() {
        let candidate = candidate.to_ascii_lowercase();
        if candidate.contains("reasoning") || candidate.contains("thought") {
            return Some(StreamTextDeltaKind::Reasoning);
        }
        if candidate.contains("assistant")
            || candidate.contains("text_delta")
            || candidate.contains("message_delta")
        {
            return Some(StreamTextDeltaKind::Assistant);
        }
    }
    None
}

fn stream_assistant_delta_text(event: &serde_json::Value) -> Option<String> {
    for pointer in [
        "/text",
        "/delta/text",
        "/delta/content",
        "/content_delta",
        "/content/text",
        "/choices/0/delta/content",
        "/message/delta/content",
    ] {
        if let Some(text) = event.pointer(pointer).and_then(|v| v.as_str()) {
            return Some(text.to_string());
        }
    }
    if let Some(delta) = event.get("delta") {
        if let Some(text) = provider_stream_text_preserving_whitespace(delta) {
            return Some(text);
        }
    }
    None
}

fn stream_reasoning_delta_text(event: &serde_json::Value) -> Option<String> {
    for pointer in [
        "/reasoning",
        "/thinking",
        "/thought",
        "/delta/reasoning",
        "/delta/reasoning_content",
        "/delta/thinking",
        "/delta/thought",
        "/choices/0/delta/reasoning",
        "/choices/0/delta/reasoning_content",
        "/choices/0/delta/thinking",
        "/message/delta/reasoning",
        "/message/delta/reasoning_content",
    ] {
        if let Some(text) = event.pointer(pointer).and_then(|v| v.as_str()) {
            return Some(text.to_string());
        }
    }
    None
}

fn pseudo_tool_call_name(text: &str) -> Option<String> {
    for name in supported_provider_tool_names() {
        if text.contains(&format!("to=functions.{name}"))
            || text.contains(&format!("functions.{name}"))
            || text.contains(&format!("<tool_call>{name}"))
        {
            return Some(name.to_string());
        }
    }
    None
}

fn native_provider_tool_request_event(
    event: &serde_json::Value,
    inner: &serde_json::Value,
    has_provider_approval_request: bool,
) -> Option<GatewayEvent> {
    native_provider_tool_request_event_with_args(
        event,
        inner,
        has_provider_approval_request,
        None,
        None,
    )
}

fn native_provider_tool_request_event_with_args(
    event: &serde_json::Value,
    inner: &serde_json::Value,
    has_provider_approval_request: bool,
    args_override: Option<serde_json::Value>,
    tool_name_override: Option<&str>,
) -> Option<GatewayEvent> {
    let tool_call = tool_call_value(inner, event);
    let tool_name = tool_name_override.or_else(|| tool_call_name(tool_call, inner, event))?;
    let client_tool = ClientToolName::from_provider_alias(tool_name);
    let den_server_tool = builtin_den_tool_descriptor_for_provider_name(tool_name).is_some();
    let unsupported_tool_detail = if client_tool.is_none() && !den_server_tool {
        let mut supported = supported_provider_tool_names()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        for descriptor in builtin_den_tool_descriptors() {
            supported.push(descriptor.provider_name);
            supported.extend(
                descriptor
                    .provider_aliases
                    .iter()
                    .map(|name| name.to_string()),
            );
        }
        Some(format!(
            "Unsupported client/Den tool: {tool_name}. Supported client/Den tools: {}.",
            supported.join(", ")
        ))
    } else {
        None
    };
    let args = if let Some(args) = args_override {
        args
    } else {
        match tool_call_args_raw(tool_call, inner, event) {
            Some(v) if !v.is_null() => {
                if let Some(s) = v.as_str() {
                    let parsed = serde_json::from_str::<serde_json::Value>(s).ok()?;
                    if parsed.is_null() {
                        return None;
                    }
                    parsed
                } else {
                    v.clone()
                }
            }
            _ => return None,
        }
    };
    if let Some(tool) = client_tool {
        let descriptor = tool.descriptor();
        if let Some(missing) = tool.missing_required_string_arg(&args) {
            if !args.is_object() || args.as_object().is_some_and(|m| m.is_empty()) {
                return None;
            }
            return Some(GatewayEvent::Error {
                message: format!(
                    "Runtime requested {} without a {missing} argument.",
                    descriptor.provider_name
                ),
                detail: Some(format!(
                    "Parsed arguments did not contain required string field `{missing}`; args={}",
                    preview_str_truncated(&args.to_string(), 240)
                )),
                error_type: Some("invalid_tool_arguments".to_string()),
                request_id: None,
                context: Some(serde_json::json!({
                    "tool_name": tool_name,
                    "tool_call_id": tool_call
                        .and_then(|v| v.get("tool_call_id"))
                        .or_else(|| tool_call.and_then(|v| v.get("id")))
                        .and_then(|v| v.as_str()),
                    "args": args,
                    "missing": missing,
                })),
            });
        }
    }
    let tool_call_id =
        tool_call_id(tool_call, inner, event).unwrap_or_else(|| format!("call-{}", Uuid::new_v4()));
    let adapter_approval_required =
        client_tool.is_some() && !den_server_tool && unsupported_tool_detail.is_none();
    let provider_approval_request_id = has_provider_approval_request.then(|| {
        // Prefer an explicit `approval_request_id` (carried by the runtime-parser seed
        // value) before the raw provider `id` field. Reading only `id` regenerated a fresh
        // UUID for the seed path, so the registered obligation's approval id no longer
        // matched the one the client echoes back, rejecting the result with a 400.
        event
            .get("approval_request_id")
            .or_else(|| event.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("approval-{}", Uuid::new_v4()))
    });
    let request_id = event
        .get("request_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let turn_id = event
        .get("turn_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let display = den_tool_display_json_for_provider(tool_name, &args)
        .unwrap_or_else(|| client_tool_display_for_provider(tool_name, &args));
    let title = display
        .get("title")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            client_tool
                .map(|tool| tool.descriptor().title.to_string())
                .unwrap_or_else(|| tool_name.to_string())
        });
    let (result_tx, result_rx) = oneshot::channel();
    Some(GatewayEvent::ToolRequest {
        request_id,
        turn_id,
        tool_call_id,
        approval_request_id: provider_approval_request_id,
        tool_name: tool_name.to_string(),
        title,
        kind: if unsupported_tool_detail.is_some() {
            "unsupported".to_string()
        } else {
            client_tool
                .map(|tool| tool.descriptor().kind.to_string())
                .unwrap_or_else(|| "server_tool".to_string())
        },
        args: if let Some(detail) = unsupported_tool_detail.as_ref() {
            let mut args = args;
            args["_unsupported_detail"] = serde_json::json!(detail);
            args
        } else {
            args
        },
        approval_required: adapter_approval_required,
        approval_reason: adapter_approval_required.then(|| {
            "BEARS requires client approval before running this local armature tool.".to_string()
        }),
        result_tx: Some(result_tx),
        result_rx: Some(result_rx),
    })
}

/// Defensive compatibility layer for provider tool-call streaming.
///
/// The preferred client adapter path uses the conversation-scoped provider messages endpoint with
/// `streaming=true` and `stream_tokens=false`, which should normally yield coherent
/// step-level tool events. Older/deployed provider builds and some provider paths may
/// still surface tool calls as repeated delta-like `approval_request_message` events:
/// the tool name can appear in one event, arguments can arrive later as string
/// fragments, and duplicate events for the same `tool_call_id` may be emitted.
///
/// Keep this accumulator even if it looks vestigial in the clean/native case. It is a
/// low-cost guardrail that reconstructs partial tool-call deltas into exactly one
/// `GatewayEvent::ToolRequest` and prevents early/duplicate local tool execution.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    names: BTreeMap<String, String>,
    argument_buffers: BTreeMap<String, String>,
    emitted: BTreeMap<String, usize>,
    openai_delta_index_ids: BTreeMap<String, String>,
}

impl ToolCallAccumulator {
    pub fn pending_argument_buffers(&self) -> usize {
        self.argument_buffers.len()
    }

    pub fn pending_name_buffers(&self) -> usize {
        self.names.len()
    }

    pub fn observe(&mut self, event: &serde_json::Value) -> Option<GatewayEvent> {
        let inner = provider_inner(event);
        let message_type = inner
            .get("message_type")
            .and_then(|v| v.as_str())
            .or_else(|| event.get("message_type").and_then(|v| v.as_str()))
            .unwrap_or("");
        if let Some(mapped) = self.observe_openai_tool_call_delta(event) {
            return Some(mapped);
        }
        if !matches!(
            message_type,
            "tool_call_message" | "approval_request_message" | "function_call"
        ) {
            return None;
        }
        let tool_call = tool_call_value(inner, event);
        let tool_call_id = tool_call_id(tool_call, inner, event)
            .unwrap_or_else(|| format!("unknown-{}", Uuid::new_v4()));
        if self.emitted.contains_key(&tool_call_id) {
            return None;
        }
        if let Some(name) = tool_call_name(tool_call, inner, event) {
            self.names.insert(tool_call_id.clone(), name.to_string());
        }
        let args = self.parse_args_fragment(&tool_call_id, tool_call, inner, event)?;
        let tool_name = self.names.get(&tool_call_id).map(String::as_str)?;
        let mapped = native_provider_tool_request_event_with_args(
            event,
            inner,
            message_type == "approval_request_message",
            Some(args),
            Some(tool_name),
        );
        if mapped.is_some() {
            self.names.remove(&tool_call_id);
            self.argument_buffers.remove(&tool_call_id);
            *self.emitted.entry(tool_call_id).or_insert(0) += 1;
        }
        mapped
    }

    fn observe_openai_tool_call_delta(
        &mut self,
        event: &serde_json::Value,
    ) -> Option<GatewayEvent> {
        let tool_call = openai_stream_tool_call_delta(event)?;
        let index_key = tool_call
            .get("index")
            .map(openai_tool_call_index_key)
            .unwrap_or_else(|| "0".to_string());
        if let Some(id) = tool_call_id(Some(tool_call), &serde_json::Value::Null, event) {
            self.openai_delta_index_ids.insert(index_key.clone(), id);
        }
        let tool_call_id = self.openai_delta_index_ids.get(&index_key)?.clone();
        if self.emitted.contains_key(&tool_call_id) {
            return None;
        }
        if let Some(name) = tool_call_name(Some(tool_call), &serde_json::Value::Null, event) {
            self.names.insert(tool_call_id.clone(), name.to_string());
        }
        let args = self.parse_args_fragment(
            &tool_call_id,
            Some(tool_call),
            &serde_json::Value::Null,
            event,
        )?;
        let tool_name = self.names.get(&tool_call_id)?.clone();
        let synthetic = serde_json::json!({
            "message_type": "function_call",
            "tool_call": {
                "name": tool_name,
                "tool_call_id": tool_call_id.clone(),
                "arguments": args,
            }
        });
        let mapped = native_provider_tool_request_event_with_args(
            &synthetic,
            &synthetic,
            false,
            Some(args),
            Some(&tool_name),
        );
        if mapped.is_some() {
            self.names.remove(&tool_call_id);
            self.argument_buffers.remove(&tool_call_id);
            self.openai_delta_index_ids.remove(&index_key);
            *self.emitted.entry(tool_call_id).or_insert(0) += 1;
        }
        mapped
    }

    fn parse_args_fragment(
        &mut self,
        tool_call_id: &str,
        tool_call: Option<&serde_json::Value>,
        inner: &serde_json::Value,
        event: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        let args_raw = tool_call_args_raw(tool_call, inner, event)?;
        if args_raw.is_null() {
            return None;
        }
        if let Some(fragment) = args_raw.as_str() {
            let buffer = self
                .argument_buffers
                .entry(tool_call_id.to_string())
                .or_default();
            buffer.push_str(fragment);
            match serde_json::from_str::<serde_json::Value>(buffer) {
                Ok(value) if !value.is_null() => Some(value),
                _ => None,
            }
        } else {
            Some(args_raw.clone())
        }
    }
}

fn openai_stream_tool_call_delta(event: &serde_json::Value) -> Option<&serde_json::Value> {
    event
        .pointer("/choices/0/delta/tool_calls")
        .and_then(|v| v.as_array())
        .and_then(|items| items.first())
        .or_else(|| {
            event
                .pointer("/delta/tool_calls")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first())
        })
}

fn openai_tool_call_index_key(value: &serde_json::Value) -> String {
    value
        .as_u64()
        .map(|index| index.to_string())
        .or_else(|| value.as_i64().map(|index| index.to_string()))
        .or_else(|| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "0".to_string())
}

fn tool_call_value<'a>(
    inner: &'a serde_json::Value,
    event: &'a serde_json::Value,
) -> Option<&'a serde_json::Value> {
    inner
        .get("tool_call")
        .or_else(|| event.get("tool_call"))
        .or_else(|| {
            inner
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first())
        })
        .or_else(|| {
            event
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first())
        })
}

fn lookup_field<'a>(
    sources: &[Option<&'a serde_json::Value>],
    paths: &[&[&str]],
) -> Option<&'a serde_json::Value> {
    sources.iter().filter_map(|source| *source).find_map(|source| {
        paths.iter().find_map(|path| {
            let mut value = source;
            for segment in *path {
                value = value.get(*segment)?;
            }
            Some(value)
        })
    })
}

fn tool_call_id(
    tool_call: Option<&serde_json::Value>,
    inner: &serde_json::Value,
    event: &serde_json::Value,
) -> Option<String> {
    lookup_field(
        &[tool_call, Some(inner), Some(event)],
        &[
            &["tool_call_id"],
            &["id"],
            &["function", "tool_call_id"],
        ],
    )
    .and_then(|v| v.as_str())
    .map(str::to_string)
}

fn tool_call_name<'a>(
    tool_call: Option<&'a serde_json::Value>,
    inner: &'a serde_json::Value,
    event: &'a serde_json::Value,
) -> Option<&'a str> {
    lookup_field(
        &[tool_call, Some(inner), Some(event)],
        &[["name"].as_slice(), &["function", "name"], &["tool_name"]],
    )
    .and_then(|v| v.as_str())
    .filter(|s| !s.is_empty())
}

fn tool_call_args_raw<'a>(
    tool_call: Option<&'a serde_json::Value>,
    inner: &'a serde_json::Value,
    event: &'a serde_json::Value,
) -> Option<&'a serde_json::Value> {
    lookup_field(
        &[tool_call, Some(inner), Some(event)],
        &[
            &["input"],
            &["arguments"],
            &["args"],
            &["function", "arguments"],
        ],
    )
}

pub fn map_provider_stream_event_to_gateway_event_with_accumulator(
    event: &serde_json::Value,
    accumulator: &mut ToolCallAccumulator,
) -> Option<GatewayEvent> {
    if let Some(mapped) = accumulator.observe(event) {
        return Some(mapped);
    }
    map_provider_stream_event_to_gateway_event(event)
}

pub fn conversation_resolved_gateway_event(event: &serde_json::Value) -> Option<GatewayEvent> {
    let conversation_id = event
        .get("conversation_id")
        .or_else(|| event.get("conversationId"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| s.starts_with("conv-"))?;
    let ty = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let message_type = event
        .get("message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if ty == "conversation_resolved" || message_type == "conversation_resolved" {
        Some(GatewayEvent::ConversationResolved {
            conversation_id: conversation_id.to_string(),
        })
    } else {
        None
    }
}

pub fn gateway_event_adapter_type(event: &GatewayEvent) -> &'static str {
    event.adapter_type()
}

pub fn gateway_event_has_visible_output(event: &GatewayEvent) -> bool {
    event.has_visible_output()
}

pub fn gateway_event_to_adapter_sse(event: GatewayEvent) -> Bytes {
    let mapped = match event {
        GatewayEvent::AssistantTextDelta { text } => serde_json::json!({
            "type": "assistant_text_delta",
            "text": text,
        }),
        GatewayEvent::ReasoningTextDelta { text } => serde_json::json!({
            "type": "reasoning_text_delta",
            "text": text,
        }),
        GatewayEvent::StatusText { text } => serde_json::json!({
            "type": "status_text",
            "text": text,
        }),
        GatewayEvent::TurnComplete { outcome } => serde_json::json!({
            "type": "turn_complete",
            "outcome": outcome,
        }),
        GatewayEvent::TurnResult {
            status,
            reason,
            request_id,
            session_id,
            retryable,
            diagnostics,
        } => serde_json::json!({
            "type": "turn_result",
            "status": status,
            "reason": reason,
            "request_id": request_id,
            "session_id": session_id,
            "retryable": retryable,
            "diagnostics": diagnostics,
        }),
        GatewayEvent::Error {
            message,
            detail,
            error_type,
            request_id,
            context,
        } => {
            let mut mapped = serde_json::json!({
                "type": "error",
                "message": message,
                "detail": detail,
                "error_type": error_type,
            });
            if let Some(context) = context {
                mapped["context"] = context;
            }
            if let Some(request_id) = request_id {
                mapped["request_id"] = serde_json::json!(request_id);
            }
            mapped
        }
        GatewayEvent::ToolRequest {
            request_id,
            turn_id,
            tool_call_id,
            approval_request_id,
            tool_name,
            title,
            kind,
            args,
            approval_required,
            approval_reason,
            result_tx: _,
            result_rx: _,
        } => {
            let display = den_tool_display_json_for_provider(&tool_name, &args)
                .unwrap_or_else(|| client_tool_display_for_provider(&tool_name, &args));
            serde_json::json!({
                "type": "tool_request",
                "request_id": request_id,
                "turn_id": turn_id,
                "tool_call_id": tool_call_id,
                "approval_request_id": approval_request_id,
                "tool_name": tool_name,
                "title": title,
                "kind": kind,
                "args": args,
                "display": display,
                "approval": {
                    "required": approval_required,
                    "reason": approval_reason,
                },
                "policy": den_tool_policy_json_for_provider(&tool_name)
                    .unwrap_or_else(|| client_tool_policy_json_for_provider(&tool_name)),
                "diagnostic": {
                    "component": "den.armature",
                    "phase": diag_phase::RUNTIME_TOOL_CALL_MAPPED,
                    "transport_version": 4,
                },
            })
        }
        GatewayEvent::PermissionRequest {
            request_id,
            permission_id,
            tool_call_id,
            tool_name,
            title,
            reason,
            target,
            options,
        } => serde_json::json!({
            "type": "permission_request",
            "request_id": request_id,
            "permission_id": permission_id,
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "title": title,
            "reason": reason,
            "target": target,
            "options": options,
            "diagnostic": {
                "component": "den.armature",
                "phase": "permission_request_mapped",
                "transport_version": 3,
            }
        }),
        GatewayEvent::SessionInfoUpdate {
            title,
            updated_at,
            meta,
        } => {
            let mut mapped = serde_json::json!({
                "type": "session_info_update",
                "title": title,
                "updated_at": updated_at,
                "diagnostic": {
                "component": "den.armature",
                    "phase": "session_info_update"
                }
            });
            if let Some(meta) = meta {
                mapped["meta"] = meta;
            }
            mapped
        }
        GatewayEvent::PlanUpdate(plan) => serde_json::json!({
            "type": "plan_update",
            "plan_id": plan.id,
            "version": plan.version,
            "title": plan.title,
            "entries": plan.items.iter().map(|item| {
                let blocked_reason = item.blocked_reason.as_deref().unwrap_or("").trim();
                let summary = item.summary.as_deref().unwrap_or("").trim();
                let content = match item.status {
                    TaskListItemStatus::Blocked if !blocked_reason.is_empty() => format!("Blocked: {} — {}", item.title, blocked_reason),
                    TaskListItemStatus::Blocked => format!("Blocked: {}", item.title),
                    TaskListItemStatus::Cancelled => format!("Cancelled: {}", item.title),
                    _ if !summary.is_empty() => format!("{} — {}", item.title, summary),
                    _ => item.title.clone(),
                };
                let status = match item.status {
                    TaskListItemStatus::InProgress => "in_progress",
                    TaskListItemStatus::Completed | TaskListItemStatus::Cancelled => "completed",
                    _ => "pending",
                };
                let priority = if item.status == TaskListItemStatus::InProgress { "high" } else { "medium" };
                serde_json::json!({
                    "content": content,
                    "priority": priority,
                    "status": status,
                    "_meta": {
                        "bears": {
                            "item_id": item.id,
                            "status": item.status.as_str(),
                            "blocked_reason": item.blocked_reason,
                            "source_refs": item.source_refs,
                        }
                    }
                })
            }).collect::<Vec<_>>(),
            "diagnostic": {
                "component": "den.armature",
                "phase": "plan_update_mapped",
                "transport_version": 3,
            }
        }),
        GatewayEvent::PlanUpdateJson { entries } => serde_json::json!({
            "type": "plan_update",
            "entries": entries,
            "diagnostic": {
                "component": "den.armature",
                "phase": "plan_update_mapped",
                "transport_version": 3,
            }
        }),
        GatewayEvent::PlanApprovalFallback {
            plan_id,
            title,
            body,
            artifact_path,
            state,
            approval_status,
        } => serde_json::json!({
            "type": "plan_update",
            "entries": [{
                "content": format!("Review submitted implementation plan: {title}"),
                "priority": "high",
                "status": "in_progress",
                "_meta": {
                    "bears": {
                        "kind": "submitted_plan_approval",
                        "plan_id": plan_id,
                        "state": state,
                        "approval_status": approval_status,
                        "artifact_path": artifact_path,
                        "title": title,
                    }
                }
            }],
            "approval_fallback": {
                "kind": "submitted_plan_approval",
                "plan_id": plan_id,
                "title": title,
                "body": body,
                "artifact_path": artifact_path,
                "state": state,
                "approval_status": approval_status,
            },
            "diagnostic": {
                "component": "den.armature",
                "phase": "plan_approval_fallback_mapped",
                "transport_version": 3,
            }
        }),
        GatewayEvent::ModeUpdate { mode } => serde_json::json!({
            "type": "mode_update",
            "mode": mode,
            "diagnostic": {
                "component": "den.armature",
                "phase": "mode_update_mapped",
                "transport_version": 3,
            }
        }),
        GatewayEvent::ConversationResolved { conversation_id } => serde_json::json!({
            "type": "conversation_resolved",
            "conversation_id": conversation_id,
        }),
    };
    Bytes::from(format!("data: {}\n\n", mapped))
}

fn preview_str_truncated(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Back off to a UTF-8 char boundary so multi-byte input can't panic.
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_call_event(name: &str, args: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "message_type": "tool_call_message",
            "tool_call": {
                "name": name,
                "tool_call_id": "call-test",
                "arguments": args.to_string(),
            }
        })
    }

    fn tool_card_payload_from_mock_provider(
        name: &str,
        args: serde_json::Value,
    ) -> serde_json::Value {
        let event = tool_call_event(name, args);
        let mapped = map_provider_stream_event_to_gateway_event(&event).expect("mapped event");
        let bytes = gateway_event_to_adapter_sse(mapped);
        let text = std::str::from_utf8(bytes.as_ref()).expect("valid utf8 sse");
        let json = text.trim().strip_prefix("data: ").expect("sse data frame");
        serde_json::from_str(json).expect("json payload")
    }

    #[test]
    fn mock_provider_to_client_card_includes_conversation_title_target() {
        let payload = tool_card_payload_from_mock_provider(
            "set_conversation_title",
            serde_json::json!({ "title": "Real Title Value" }),
        );

        assert_eq!(payload["type"], "tool_request");
        let title = payload["title"].as_str().expect("title string");
        assert!(title.contains("Real Title Value"), "{payload}");
        assert!(!title.ends_with(": conversation"), "{payload}");
        assert_eq!(payload["display"]["subtitle"], "Real Title Value");
    }

    #[test]
    fn mock_provider_to_client_card_includes_run_command_args() {
        let payload = tool_card_payload_from_mock_provider(
            "run_command",
            serde_json::json!({
                "command": "git",
                "args": ["status", "--short"],
                "cwd": "/workspace/services/den"
            }),
        );

        assert_eq!(payload["type"], "tool_request");
        let title = payload["title"].as_str().expect("title string");
        assert!(title.contains("git status --short"), "{payload}");
        assert_ne!(title, "Run command", "{payload}");
        assert_eq!(
            payload["display"]["subtitle"],
            "git status --short → …/workspace/services/den"
        );
    }

    #[test]
    fn mock_provider_to_client_card_includes_create_job_goal_without_branding() {
        let payload = tool_card_payload_from_mock_provider(
            "create_job",
            serde_json::json!({
                "goal": "Fix ACP tool-card display summaries",
                "tasks": []
            }),
        );

        assert_eq!(payload["type"], "tool_request");
        let title = payload["title"].as_str().expect("title string");
        assert!(
            title.contains("Fix ACP tool-card display summaries"),
            "{payload}"
        );
        assert!(title.contains("job"), "{payload}");
        assert!(!title.contains("Docket"), "{payload}");
        assert_eq!(
            payload["display"]["subtitle"],
            "Fix ACP tool-card display summaries"
        );
    }

    #[test]
    fn maps_list_directory_tool_call() {
        let event = tool_call_event(
            "fs_list_directory",
            serde_json::json!({ "path": "/workspace" }),
        );
        let mapped = map_provider_stream_event_to_gateway_event(&event).expect("mapped event");
        match mapped {
            GatewayEvent::ToolRequest {
                tool_name,
                kind,
                args,
                ..
            } => {
                assert_eq!(tool_name, "fs_list_directory");
                assert_eq!(kind, "read");
                assert_eq!(args["path"], "/workspace");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_search_files_tool_call() {
        let event = tool_call_event(
            "fs_search_files",
            serde_json::json!({ "path": "/workspace", "query": "needle" }),
        );
        let mapped = map_provider_stream_event_to_gateway_event(&event).expect("mapped event");
        match mapped {
            GatewayEvent::ToolRequest {
                tool_name,
                kind,
                args,
                ..
            } => {
                assert_eq!(tool_name, "fs_search_files");
                assert_eq!(kind, "search");
                assert_eq!(args["path"], "/workspace");
                assert_eq!(args["query"], "needle");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn tool_call_message_requires_adapter_approval_without_provider_approval_id() {
        let event = tool_call_event(
            "fs_edit_file",
            serde_json::json!({
                "path": "/workspace/a.txt",
                "old_text": "before",
                "new_text": "after"
            }),
        );
        let mapped = map_provider_stream_event_to_gateway_event(&event).expect("mapped event");
        match mapped {
            GatewayEvent::ToolRequest {
                approval_required,
                approval_request_id,
                approval_reason,
                ..
            } => {
                assert!(approval_required);
                assert!(approval_request_id.is_none());
                assert!(approval_reason.is_some());
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn seed_path_approval_request_id_is_preserved_not_regenerated() {
        // The runtime-parser seed value is flat (top-level `tool_call_id`/`tool_name`/`args`/
        // `approval_request_id`, no raw `id`) and is mapped through the accumulator. The mapper
        // must use `approval_request_id` verbatim; reading only `id` regenerated a fresh UUID,
        // so the registered obligation's approval id no longer matched the one the client
        // echoes back and the tool result was rejected with a 400.
        let event = serde_json::json!({
            "message_type": "approval_request_message",
            "tool_call_id": "call-seed",
            "tool_name": "fs_read_text_file",
            "args": { "path": "/workspace/a.txt" },
            "approval_request_id": "approval-call-seed",
        });
        let mut accumulator = ToolCallAccumulator::default();
        let mapped =
            map_provider_stream_event_to_gateway_event_with_accumulator(&event, &mut accumulator)
                .expect("mapped event");
        match mapped {
            GatewayEvent::ToolRequest {
                approval_required,
                approval_request_id,
                ..
            } => {
                assert!(approval_required);
                assert_eq!(approval_request_id.as_deref(), Some("approval-call-seed"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn raw_path_approval_request_id_falls_back_to_id() {
        // Compatibility: the raw provider SSE nests identity under `tool_call` and carries the
        // approval id in top-level `id`.
        let event = serde_json::json!({
            "id": "approval-raw",
            "message_type": "approval_request_message",
            "tool_call": {
                "name": "fs_read_text_file",
                "tool_call_id": "call-raw",
                "arguments": serde_json::json!({ "path": "/workspace/a.txt" }).to_string(),
            },
        });
        let mut accumulator = ToolCallAccumulator::default();
        let mapped =
            map_provider_stream_event_to_gateway_event_with_accumulator(&event, &mut accumulator)
                .expect("mapped event");
        match mapped {
            GatewayEvent::ToolRequest {
                approval_request_id,
                ..
            } => {
                assert_eq!(approval_request_id.as_deref(), Some("approval-raw"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_replace_text_tool_call() {
        let event = tool_call_event(
            "fs_edit_file",
            serde_json::json!({
                "path": "/workspace/a.txt",
                "old_text": "before",
                "new_text": "after"
            }),
        );
        let mapped = map_provider_stream_event_to_gateway_event(&event).expect("mapped event");
        match mapped {
            GatewayEvent::ToolRequest {
                tool_name,
                kind,
                args,
                ..
            } => {
                assert_eq!(tool_name, "fs_edit_file");
                assert_eq!(kind, "edit");
                assert_eq!(args["old_text"], "before");
                assert_eq!(args["new_text"], "after");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn search_files_requires_query() {
        let event = tool_call_event(
            "fs_search_files",
            serde_json::json!({ "path": "/workspace" }),
        );
        let mapped = map_provider_stream_event_to_gateway_event(&event).expect("mapped event");
        match mapped {
            GatewayEvent::Error {
                error_type,
                message,
                context,
                ..
            } => {
                assert_eq!(error_type.as_deref(), Some("invalid_tool_arguments"));
                assert!(message.contains("fs_search_files"));
                assert_eq!(context.unwrap()["missing"], "query");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn replace_text_requires_new_text() {
        let event = tool_call_event(
            "fs_edit_file",
            serde_json::json!({ "path": "/workspace/a.txt", "old_text": "before" }),
        );
        let mapped = map_provider_stream_event_to_gateway_event(&event).expect("mapped event");
        match mapped {
            GatewayEvent::Error {
                error_type,
                message,
                context,
                ..
            } => {
                assert_eq!(error_type.as_deref(), Some("invalid_tool_arguments"));
                assert!(message.contains("fs_edit_file"));
                assert_eq!(context.unwrap()["missing"], "new_text");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_openai_style_assistant_delta() {
        let event = serde_json::json!({
            "type": "message_delta",
            "choices": [{ "delta": { "role": "assistant", "content": "hello" } }]
        });
        match map_provider_stream_event_to_gateway_event(&event) {
            Some(GatewayEvent::AssistantTextDelta { text }) => assert_eq!(text, "hello"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_openai_style_assistant_delta_without_repeated_role() {
        let event = serde_json::json!({
            "type": "chat.completion.chunk",
            "choices": [{ "delta": { "content": " world" } }]
        });
        match map_provider_stream_event_to_gateway_event(&event) {
            Some(GatewayEvent::AssistantTextDelta { text }) => assert_eq!(text, " world"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_reasoning_delta_fallback() {
        let event = serde_json::json!({
            "type": "reasoning_delta",
            "delta": { "text": "thinking" }
        });
        match map_provider_stream_event_to_gateway_event(&event) {
            Some(GatewayEvent::ReasoningTextDelta { text }) => assert_eq!(text, "thinking"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_openai_style_reasoning_content_delta() {
        let event = serde_json::json!({
            "type": "chat.completion.chunk",
            "choices": [{ "delta": { "reasoning_content": "thinking" } }]
        });
        match map_provider_stream_event_to_gateway_event(&event) {
            Some(GatewayEvent::ReasoningTextDelta { text }) => assert_eq!(text, "thinking"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn explicit_reasoning_field_beats_generic_message_delta_kind() {
        let event = serde_json::json!({
            "type": "message_delta",
            "delta": { "reasoning_content": "thinking" }
        });
        match map_provider_stream_event_to_gateway_event(&event) {
            Some(GatewayEvent::ReasoningTextDelta { text }) => assert_eq!(text, "thinking"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn accumulates_openai_style_tool_call_deltas() {
        let mut accumulator = ToolCallAccumulator::default();
        let first = serde_json::json!({
            "type": "chat.completion.chunk",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_web_fetch",
                        "type": "function",
                        "function": {
                            "name": "web_fetch",
                            "arguments": "{\"url\":\"https://exa"
                        }
                    }]
                }
            }]
        });
        assert!(map_provider_stream_event_to_gateway_event_with_accumulator(
            &first,
            &mut accumulator
        )
        .is_none());

        let second = serde_json::json!({
            "type": "chat.completion.chunk",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {
                            "arguments": "mple.com/docs\"}"
                        }
                    }]
                }
            }]
        });
        match map_provider_stream_event_to_gateway_event_with_accumulator(&second, &mut accumulator)
        {
            Some(GatewayEvent::ToolRequest {
                tool_call_id,
                tool_name,
                args,
                approval_required,
                ..
            }) => {
                assert_eq!(tool_call_id, "call_web_fetch");
                assert_eq!(tool_name, "web_fetch");
                assert_eq!(args["url"], "https://example.com/docs");
                assert!(!approval_required);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn detects_pseudo_tool_call_text() {
        let event = serde_json::json!({
            "message_type": "assistant_message",
            "content": "to=functions.fs_edit_file {\"path\":\"/workspace/README.md\"}"
        });
        let mapped = map_provider_stream_event_to_gateway_event(&event).expect("mapped event");
        match mapped {
            GatewayEvent::Error {
                error_type,
                context,
                ..
            } => {
                assert_eq!(error_type.as_deref(), Some("pseudo_tool_call_text"));
                assert_eq!(context.unwrap()["tool_name"], "fs_edit_file");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn tool_return_message_is_diagnostic_only() {
        let event = serde_json::json!({
            "message_type": "tool_return_message",
            "tool_call_id": "call-1",
            "status": "success",
            "tool_return": "hello"
        });
        assert!(map_provider_stream_event_to_gateway_event(&event).is_none());
    }

    #[test]
    fn maps_tool_call_to_adapter_sse_without_database() {
        let event = tool_call_event(
            "fs_edit_file",
            serde_json::json!({
                "path": "/workspace/a.txt",
                "old_text": "before",
                "new_text": "after"
            }),
        );
        let mapped = map_provider_stream_event_to_gateway_event(&event).expect("mapped event");
        let bytes = gateway_event_to_adapter_sse(mapped);
        let raw = std::str::from_utf8(&bytes).expect("utf8 sse");
        assert!(raw.contains("\"type\":\"tool_request\""));
        assert!(raw.contains("\"tool_name\":\"fs_edit_file\""));
        assert!(raw.contains("\"required\":true"));
        assert!(raw.contains("\"risk\":\"writes_workspace\""));
        assert!(raw.contains("\"phase\":\"runtime_tool_call_mapped\""));
    }

    #[test]
    fn list_directory_sse_policy_includes_entry_limit() {
        let event = GatewayEvent::ToolRequest {
            request_id: "request-1".to_string(),
            turn_id: "turn-1".to_string(),
            tool_call_id: "call-1".to_string(),
            approval_request_id: None,
            tool_name: "fs_list_directory".to_string(),
            title: "List directory".to_string(),
            kind: "read".to_string(),
            args: serde_json::json!({ "path": "/workspace" }),
            approval_required: false,
            approval_reason: None,
            result_tx: None,
            result_rx: None,
        };
        let bytes = gateway_event_to_adapter_sse(event);
        let raw = std::str::from_utf8(&bytes).expect("utf8 sse");
        assert!(raw.contains("\"max_entries\":1000"));
        assert!(raw.contains("\"risk\":\"read_only\""));
    }
}
