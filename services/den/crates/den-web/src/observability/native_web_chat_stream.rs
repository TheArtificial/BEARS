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
        RuntimeSemanticEvent::StatusText { text } => {
            vec![serde_json::json!({ "type": "reasoning_delta", "text": text })]
        }
        RuntimeSemanticEvent::ToolCallFinished {
            tool_name,
            status,
            summary,
            error_message,
            ..
        } => {
            let display_summary = summary
                
                .or_else(|| error_message.clone())
                .unwrap_or_else(|| format!("Finished {tool_name}"));
            let mut events = vec![serde_json::json!({
                "type": "server_tool_finished",
                "tool": tool_name,
                "summary": display_summary,
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
            text: Some(text),
            ..
        } => {
            if is_ephemeral_progress_status(&text) {
                vec![serde_json::json!({ "type": "status_progress", "text": text })]
            } else {
                vec![serde_json::json!({ "type": "reasoning_delta", "text": text })]
            }
        }
        RuntimeSemanticEvent::RunProgress { kind, text: None, .. } => {
            if is_ephemeral_progress_status(&kind) {
                vec![serde_json::json!({ "type": "status_progress", "text": kind })]
            } else {
                vec![serde_json::json!({ "type": "reasoning_delta", "text": kind })]
            }
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
            tool_name,
            title,
            ..
        } => {
            let summary = title.unwrap_or_else(|| tool_name.clone());
            vec![serde_json::json!({
                "type": "server_tool_started",
                "tool": summary,
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
            category,
            message,
            ..
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

fn runtime_stream_event_to_bear_channel_bytes(
    event: RuntimeStreamEvent,
    request_id: Option<&str>,
) -> Vec<Bytes> {
    match event {
        RuntimeStreamEvent::Semantic(semantic) => {
            runtime_semantic_to_bear_channel_events(semantic, request_id)
                .into_iter()
                .filter_map(|value| bear_channel_sse_bytes(&value))
                .collect()
        }
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => Vec::new(),
    }
}

/// Presents native runtime events as bear_channel SSE bytes for [`super::chat_proxy_stream::BearChannelSseProxyStream`].
pub struct NativeWebChatUpstreamStream {
    inner: Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, crate::errors::CustomError>> + Send>>,
    request_id: Uuid,
    pending: VecDeque<Bytes>,
    finished: bool,
}

impl NativeWebChatUpstreamStream {
    pub fn new(
        inner: impl Stream<Item = Result<RuntimeStreamEvent, crate::errors::CustomError>> + Send + 'static,
        request_id: Uuid,
    ) -> Self {
        Self {
            inner: Box::pin(inner),
            request_id,
            pending: VecDeque::new(),
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
        if let Some(bytes) = bear_channel_sse_bytes(&value) {
            self.pending.push_back(bytes);
        }
        if let Some(bytes) = bear_channel_sse_bytes(&serde_json::json!({ "type": "done", "outcome": "error" })) {
            self.pending.push_back(bytes);
        }
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
                    let mut bytes =
                        runtime_stream_event_to_bear_channel_bytes(event, Some(&request_id));
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
                    if let Some(bytes) =
                        bear_channel_sse_bytes(&serde_json::json!({ "type": "done", "outcome": "ok" }))
                    {
                        this.finished = true;
                        return Poll::Ready(Some(Ok(bytes)));
                    }
                    this.finished = true;
                    return Poll::Ready(None);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::chat_proxy_stream::EPHEMERAL_PROGRESS_STATUSES;

    #[test]
    fn maps_assistant_text_to_assistant_delta() {
        let events = runtime_semantic_to_bear_channel_events(
            RuntimeSemanticEvent::AssistantTextDelta {
                text: "Hello".to_string(),
            },
            None,
        );
        assert_eq!(events[0]["type"], "assistant_delta");
        assert_eq!(events[0]["text"], "Hello");
    }

    #[test]
    fn maps_ephemeral_run_progress_to_status_progress() {
        for status in EPHEMERAL_PROGRESS_STATUSES {
            let events = runtime_semantic_to_bear_channel_events(
                RuntimeSemanticEvent::RunProgress {
                    kind: "status".to_string(),
                    text: Some((*status).to_string()),
                    phase: None,
                    detail: None,
                },
                None,
            );
            assert_eq!(events[0]["type"], "status_progress", "status={status}");
            assert_eq!(events[0]["text"], *status);
        }
    }

    #[test]
    fn maps_non_ephemeral_run_progress_to_reasoning_delta() {
        let events = runtime_semantic_to_bear_channel_events(
            RuntimeSemanticEvent::RunProgress {
                kind: "status".to_string(),
                text: Some("Indexing documents".to_string()),
                phase: None,
                detail: None,
            },
            None,
        );
        assert_eq!(events[0]["type"], "reasoning_delta");
    }

    #[test]
    fn maps_status_text_to_reasoning_delta() {
        let events = runtime_semantic_to_bear_channel_events(
            RuntimeSemanticEvent::StatusText {
                text: "Indexing".to_string(),
            },
            None,
        );
        assert_eq!(events[0]["type"], "reasoning_delta");
    }

    #[test]
    fn maps_turn_completed_to_done() {
        let events = runtime_semantic_to_bear_channel_events(
            RuntimeSemanticEvent::TurnCompleted { turn: None },
            None,
        );
        assert_eq!(events[0]["type"], "done");
    }

    #[test]
    fn maps_tool_call_finished_to_status_and_single_error() {
        let events = runtime_semantic_to_bear_channel_events(
            RuntimeSemanticEvent::ToolCallFinished {
                tool_call_id: "call_1".to_string(),
                tool_name: "memory_read".to_string(),
                status: ToolCallFinishStatus::Error,
                summary: Some("memory_read failed: not found".to_string()),
                error_message: Some("memory_read failed: not found".to_string()),
            },
            Some("req-1"),
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "server_tool_finished");
        assert_eq!(events[0]["tool"], "memory_read");
        assert_eq!(events[1]["type"], "error");
        assert_eq!(events[1]["error_type"], "tool_execution_error");
    }

    #[test]
    fn maps_tool_call_requested_to_server_tool_started() {
        let events = runtime_semantic_to_bear_channel_events(
            RuntimeSemanticEvent::ToolCallRequested {
                tool_call_id: "call_1".to_string(),
                tool_name: "den_capabilities_list_self".to_string(),
                title: Some("den_capabilities_list_self".to_string()),
                kind: Some("function".to_string()),
                arguments: serde_json::json!({}),
                approval_request_id: None,
                approval_required: false,
                approval_reason: None,
                run_id: None,
            },
            None,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "server_tool_started");
        assert_eq!(events[0]["tool"], "den_capabilities_list_self");
    }

    #[test]
    fn turn_failed_includes_request_id() {
        let events = runtime_semantic_to_bear_channel_events(
            RuntimeSemanticEvent::TurnFailed {
                turn: None,
                category: RuntimeErrorCategory::Timeout,
                message: "timed out".to_string(),
            },
            Some("req-123"),
        );
        assert_eq!(events[0]["request_id"], "req-123");
        assert_eq!(events[0]["error_type"], "runtime_timeout");
    }
}
