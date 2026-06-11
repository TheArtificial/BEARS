//! Adapts Den-native runtime semantic events into bear_channel SSE for web chat.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::{ready, Stream};

use crate::core::runtime_contracts::{
    RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent,
};

use super::chat_proxy_stream::bear_channel_sse_bytes;

/// Maps a native runtime semantic event to zero or more bear_channel JSON events.
pub fn runtime_semantic_to_bear_channel_events(event: RuntimeSemanticEvent) -> Vec<serde_json::Value> {
    match event {
        RuntimeSemanticEvent::AssistantTextDelta { text } => {
            vec![serde_json::json!({ "type": "assistant_delta", "text": text })]
        }
        RuntimeSemanticEvent::StatusText { text } | RuntimeSemanticEvent::RunProgress {
            text: Some(text),
            ..
        } => vec![serde_json::json!({ "type": "reasoning_delta", "text": text })],
        RuntimeSemanticEvent::RunProgress { kind, text: None, .. } => {
            vec![serde_json::json!({ "type": "reasoning_delta", "text": kind })]
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
                "tool": tool_name,
                "summary": summary,
            })]
        }
        RuntimeSemanticEvent::Error {
            message,
            detail,
            error_type,
            request_id,
            context,
        } => vec![serde_json::json!({
            "type": "error",
            "message": message,
            "detail": detail,
            "error_type": error_type,
            "request_id": request_id,
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
        })],
        RuntimeSemanticEvent::TurnCancelled { .. } => vec![serde_json::json!({
            "type": "error",
            "message": "Runtime continuation was cancelled.",
            "error_type": "runtime_turn_cancelled",
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

fn runtime_stream_event_to_bear_channel_bytes(event: RuntimeStreamEvent) -> Vec<Bytes> {
    match event {
        RuntimeStreamEvent::Semantic(semantic) => runtime_semantic_to_bear_channel_events(semantic)
            .into_iter()
            .filter_map(|value| bear_channel_sse_bytes(&value))
            .collect(),
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => Vec::new(),
    }
}

/// Presents native runtime events as bear_channel SSE bytes for [`super::chat_proxy_stream::BearChannelSseProxyStream`].
pub struct NativeWebChatUpstreamStream {
    inner: Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, crate::errors::CustomError>> + Send>>,
    pending: VecDeque<Bytes>,
    finished: bool,
}

impl NativeWebChatUpstreamStream {
    pub fn new(
        inner: impl Stream<Item = Result<RuntimeStreamEvent, crate::errors::CustomError>> + Send + 'static,
    ) -> Self {
        Self {
            inner: Box::pin(inner),
            pending: VecDeque::new(),
            finished: false,
        }
    }

    fn push_error_bytes(&mut self, message: impl Into<String>) {
        let value = serde_json::json!({
            "type": "error",
            "message": message.into(),
            "error_type": "runtime_internal",
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

        loop {
            match ready!(this.inner.as_mut().poll_next(cx)) {
                Some(Ok(event)) => {
                    let mut bytes = runtime_stream_event_to_bear_channel_bytes(event);
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
                    this.push_error_bytes(error.to_string());
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

    #[test]
    fn maps_assistant_text_to_assistant_delta() {
        let events = runtime_semantic_to_bear_channel_events(RuntimeSemanticEvent::AssistantTextDelta {
            text: "Hello".to_string(),
        });
        assert_eq!(events[0]["type"], "assistant_delta");
        assert_eq!(events[0]["text"], "Hello");
    }

    #[test]
    fn maps_status_text_to_reasoning_delta() {
        let events = runtime_semantic_to_bear_channel_events(RuntimeSemanticEvent::StatusText {
            text: "Indexing".to_string(),
        });
        assert_eq!(events[0]["type"], "reasoning_delta");
    }

    #[test]
    fn maps_turn_completed_to_done() {
        let events =
            runtime_semantic_to_bear_channel_events(RuntimeSemanticEvent::TurnCompleted { turn: None });
        assert_eq!(events[0]["type"], "done");
    }
}
