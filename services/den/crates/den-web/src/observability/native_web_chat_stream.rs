//! Adapts Den-native runtime semantic events into bear_channel SSE for web chat.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::{ready, Stream};
use uuid::Uuid;

use den_protocol::{
    RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent, ToolCallFinishStatus,
};

use super::chat_proxy_stream::{bear_channel_sse_bytes, is_ephemeral_progress_status};

/// Maps a native runtime semantic event to zero or more bear_channel JSON events.
pub fn runtime_semantic_to_bear_channel_events(
    event: RuntimeSemanticEvent,
    request_id: Option<&str>,
) -> Vec<serde_json::Value> {
    match event {
        RuntimeSemanticEvent::AssistantTextDelta { text } => {
            vec![serde_json::json!({ "type": "assistant_delta", "text": text })]
        }
        RuntimeSemanticEvent::ReasoningTextDelta { text }
        | RuntimeSemanticEvent::StatusText { text } => {
            vec![serde_json::json!({ "type": "reasoning_delta", "text": text })]
        }
        RuntimeSemanticEvent::ToolCallFinished {
            tool_call_id,
            tool_name,
            status,
            summary,
            error_message,
        } => {
            let display_summary = summary
                .or_else(|| error_message.clone())
                .unwrap_or_else(|| format!("Finished {tool_name}"));
            let mut events = vec![serde_json::json!({
                "type": "runtime_card",
                "card_kind": "tool_activity",
                "label": display_summary,
                "source": "den_runtime",
                "tool": {
                    "id": tool_call_id,
                    "name": tool_name,
                    "status": status.as_str(),
                    "summary": display_summary,
                    "error_message": error_message,
                },
                "delivery": {
                    "persisted": true,
                    "visible_to_user": true,
                    "sent_to_model": false,
                    "derived_context": false,
                },
                "redaction": { "state": "none" },
            })];
            if status == ToolCallFinishStatus::Error {
                if let Some(message) = error_message {
                    events.push(serde_json::json!({
                        "type": "error",
                        "message": message,
                        "detail": format!("Tool `{tool_name}` returned an error."),
                        "error_type": "tool_execution_error",
                        "request_id": request_id,
                    }));
                }
            }
            events
        }
        RuntimeSemanticEvent::RunProgress {
            text: Some(text), ..
        } => {
            if is_ephemeral_progress_status(&text) {
                vec![serde_json::json!({ "type": "status_progress", "text": text })]
            } else {
                vec![serde_json::json!({ "type": "reasoning_delta", "text": text })]
            }
        }
        RuntimeSemanticEvent::RunProgress {
            kind, text: None, ..
        } => {
            if is_ephemeral_progress_status(&kind) {
                vec![serde_json::json!({ "type": "status_progress", "text": kind })]
            } else {
                vec![serde_json::json!({ "type": "reasoning_delta", "text": kind })]
            }
        }
        RuntimeSemanticEvent::BoundedSlice { reason } => {
            vec![serde_json::json!({
                "type": "status_progress",
                "text": format!("Continuing: {reason}"),
            })]
        }
        RuntimeSemanticEvent::ConversationResolved { conversation } => {
            vec![serde_json::json!({
                "type": "conversation_resolved",
                "conversation_id": conversation.id,
            })]
        }
        RuntimeSemanticEvent::TurnCompleted { .. } => {
            vec![serde_json::json!({ "type": "done", "outcome": "ok" })]
        }
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            title,
            kind,
            arguments,
            approval_request_id,
            approval_required,
            approval_reason,
            run_id,
        } => {
            let label = title.unwrap_or_else(|| format!("Run {tool_name}"));
            vec![serde_json::json!({
                "type": "runtime_card",
                "card_kind": "tool_activity",
                "label": label,
                "source": "den_runtime",
                "tool": {
                    "id": tool_call_id,
                    "name": tool_name,
                    "kind": kind,
                    "arguments": arguments,
                    "status": "requested",
                    "approval_required": approval_required,
                    "approval_request_id": approval_request_id,
                    "approval_reason": approval_reason,
                },
                "run_id": run_id,
                "delivery": {
                    "persisted": true,
                    "visible_to_user": true,
                    "sent_to_model": false,
                    "derived_context": false,
                },
                "redaction": { "state": "none" },
            })]
        }
        RuntimeSemanticEvent::Error {
            message,
            detail,
            error_type,
            request_id: event_request_id,
            context,
        } => vec![serde_json::json!({
            "type": "error",
            "message": message,
            "detail": detail,
            "error_type": error_type,
            "request_id": event_request_id.or_else(|| request_id.map(str::to_string)),
            "context": context,
        })],
        RuntimeSemanticEvent::TurnFailed {
            category, message, ..
        } => vec![serde_json::json!({
            "type": "error",
            "message": message,
            "error_type": match category {
                RuntimeErrorCategory::Unavailable => "runtime_unavailable",
                RuntimeErrorCategory::Misconfigured => "runtime_misconfigured",
                RuntimeErrorCategory::InvalidIdentity => "runtime_invalid_identity",
                RuntimeErrorCategory::PermissionDenied => "runtime_permission_denied",
                RuntimeErrorCategory::ConflictPendingApproval => "runtime_conflict_pending_approval",
                RuntimeErrorCategory::Cancelled => "runtime_cancelled",
                RuntimeErrorCategory::Timeout => "runtime_timeout",
                RuntimeErrorCategory::BackendProtocol => "runtime_backend_protocol",
                RuntimeErrorCategory::Internal => "runtime_internal",
            },
            "request_id": request_id,
        })],
        RuntimeSemanticEvent::TurnCancelled { .. } => vec![serde_json::json!({
            "type": "error",
            "message": "Runtime continuation was cancelled.",
            "error_type": "runtime_turn_cancelled",
            "request_id": request_id,
        })],
        RuntimeSemanticEvent::RunPaused { reason, .. } => {
            let text = if reason == "awaiting_approval" {
                "Waiting for approval.".to_string()
            } else {
                format!("Paused: {reason}")
            };
            vec![serde_json::json!({ "type": "reasoning_delta", "text": text })]
        }
    }
}

#[cfg(test)]
fn runtime_stream_event_to_bear_channel_bytes(
    event: RuntimeStreamEvent,
    request_id: Option<&str>,
) -> Vec<Bytes> {
    match event {
        RuntimeStreamEvent::Semantic(semantic) => {
            runtime_semantic_to_bear_channel_events(semantic, request_id)
                .into_iter()
                .map(|value| bear_channel_sse_bytes(&value))
                .collect()
        }
        RuntimeStreamEvent::ProviderActivity
        | RuntimeStreamEvent::UntranslatedProviderEvent { .. } => Vec::new(),
    }
}

/// Presents native runtime events as bear_channel SSE bytes for [`super::chat_proxy_stream::BearChannelSseProxyStream`].
pub struct NativeWebChatUpstreamStream {
    inner:
        Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, crate::errors::CustomError>> + Send>>,
    request_id: Uuid,
    pending: VecDeque<Bytes>,
    runtime_event_sequence: u64,
    finished: bool,
}

impl NativeWebChatUpstreamStream {
    pub fn new(
        inner: impl Stream<Item = Result<RuntimeStreamEvent, crate::errors::CustomError>>
            + Send
            + 'static,
        request_id: Uuid,
    ) -> Self {
        Self {
            inner: Box::pin(inner),
            request_id,
            pending: VecDeque::new(),
            runtime_event_sequence: 0,
            finished: false,
        }
    }

    fn push_error_bytes(&mut self, message: impl Into<String>, error_type: &str) {
        let value = serde_json::json!({
            "type": "error",
            "message": message.into(),
            "error_type": error_type,
            "request_id": self.request_id.to_string(),
        });
        self.pending.push_back(bear_channel_sse_bytes(&value));
        self.pending.push_back(bear_channel_sse_bytes(
            &serde_json::json!({ "type": "done", "outcome": "error" }),
        ));
        self.finished = true;
    }
}

impl Stream for NativeWebChatUpstreamStream {
    type Item = Result<Bytes, reqwest::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        if let Some(bytes) = this.pending.pop_front() {
            return Poll::Ready(Some(Ok(bytes)));
        }
        if this.finished {
            return Poll::Ready(None);
        }

        let request_id = this.request_id.to_string();
        loop {
            match ready!(this.inner.as_mut().poll_next(cx)) {
                Some(Ok(event)) => {
                    let mut values = match event {
                        RuntimeStreamEvent::Semantic(semantic) => {
                            runtime_semantic_to_bear_channel_events(semantic, Some(&request_id))
                        }
                        RuntimeStreamEvent::ProviderActivity
                        | RuntimeStreamEvent::UntranslatedProviderEvent { .. } => Vec::new(),
                    };
                    for value in &mut values {
                        if value.get("type").and_then(|kind| kind.as_str()) == Some("runtime_card") {
                            this.runtime_event_sequence += 1;
                            value["invocation_id"] = serde_json::json!(request_id);
                            value["ordering"] = serde_json::json!({
                                "stream_sequence": this.runtime_event_sequence,
                            });
                        }
                    }
                    let mut bytes = values
                        .into_iter()
                        .map(|value| bear_channel_sse_bytes(&value))
                        .collect::<Vec<_>>();
                    if bytes.is_empty() {
                        continue;
                    }
                    let first = bytes.remove(0);
                    for chunk in bytes {
                        this.pending.push_back(chunk);
                    }
                    return Poll::Ready(Some(Ok(first)));
                }
                Some(Err(error)) => {
                    this.push_error_bytes(error.to_string(), "runtime_internal");
                    if let Some(bytes) = this.pending.pop_front() {
                        return Poll::Ready(Some(Ok(bytes)));
                    }
                    return Poll::Ready(None);
                }
                None => {
                    let bytes = bear_channel_sse_bytes(
                        &serde_json::json!({ "type": "done", "outcome": "ok" }),
                    );
                    this.finished = true;
                    return Poll::Ready(Some(Ok(bytes)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
