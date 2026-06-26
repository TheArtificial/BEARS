use bytes::Bytes;

pub mod wire;

use crate::{
    gateway_events::{gateway_event_to_adapter_sse, GatewayEvent},
    runtime_contracts::{
        RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent, ToolCallFinishStatus,
    },
};

#[derive(Debug)]
pub enum RuntimeEventProjectionOutcome {
    Events(Vec<GatewayEvent>),
    Ignored { reason: &'static str },
}

pub fn project_runtime_event_lossy(event: RuntimeStreamEvent) -> RuntimeEventProjectionOutcome {
    match event {
        RuntimeStreamEvent::Semantic(event) => RuntimeEventProjectionOutcome::Events(
            runtime_semantic_event_to_bearwire_gateway_events(event),
        ),
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => RuntimeEventProjectionOutcome::Ignored {
            reason: "untranslated_provider_event",
        },
    }
}

pub fn runtime_semantic_event_to_bearwire_gateway_events(
    event: RuntimeSemanticEvent,
) -> Vec<GatewayEvent> {
    match event {
        RuntimeSemanticEvent::AssistantTextDelta { text } => {
            vec![GatewayEvent::AssistantTextDelta { text }]
        }
        RuntimeSemanticEvent::StatusText { text } => vec![GatewayEvent::StatusText { text }],
        RuntimeSemanticEvent::ConversationResolved { conversation } => {
            vec![GatewayEvent::ConversationResolved {
                conversation_id: conversation.id,
            }]
        }
        RuntimeSemanticEvent::TurnCompleted { .. } => vec![GatewayEvent::TurnComplete {
            outcome: "ok".to_string(),
        }],
        RuntimeSemanticEvent::RunPaused { reason, .. } => vec![GatewayEvent::StatusText {
            text: if reason == "awaiting_approval" {
                "Waiting for approval.".to_string()
            } else {
                format!("Paused: {reason}")
            },
        }],
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            title,
            kind,
            arguments,
            approval_request_id,
            approval_required,
            approval_reason,
            run_id: _,
        } => vec![GatewayEvent::ToolRequest {
            request_id: approval_request_id
                .clone()
                .unwrap_or_else(|| format!("runtime-tool-{tool_call_id}")),
            turn_id: "runtime-semantic".to_string(),
            tool_call_id,
            approval_request_id,
            tool_name: tool_name.clone(),
            title: title.unwrap_or_else(|| tool_name.clone()),
            kind: kind.unwrap_or_else(|| "function".to_string()),
            args: arguments,
            approval_required,
            approval_reason,
            result_tx: None,
            result_rx: None,
        }],
        RuntimeSemanticEvent::Error {
            message,
            detail,
            error_type,
            request_id,
            context,
        } => vec![GatewayEvent::Error {
            message,
            detail,
            error_type,
            request_id,
            context,
        }],
        RuntimeSemanticEvent::TurnFailed {
            category,
            message,
            ..
        } => vec![GatewayEvent::Error {
            message,
            detail: None,
            error_type: Some(match category {
                RuntimeErrorCategory::Unavailable => "runtime_unavailable",
                RuntimeErrorCategory::Misconfigured => "runtime_misconfigured",
                RuntimeErrorCategory::InvalidIdentity => "runtime_invalid_identity",
                RuntimeErrorCategory::PermissionDenied => "runtime_permission_denied",
                RuntimeErrorCategory::ConflictPendingApproval => "runtime_conflict_pending_approval",
                RuntimeErrorCategory::Cancelled => "runtime_cancelled",
                RuntimeErrorCategory::Timeout => "runtime_timeout",
                RuntimeErrorCategory::BackendProtocol => "runtime_backend_protocol",
                RuntimeErrorCategory::Internal => "runtime_internal",
            }
            .to_string()),
            request_id: None,
            context: None,
        }],
        RuntimeSemanticEvent::TurnCancelled { .. } => vec![GatewayEvent::Error {
            message: "Runtime continuation was cancelled.".to_string(),
            detail: None,
            error_type: Some("runtime_turn_cancelled".to_string()),
            request_id: None,
            context: None,
        }],
        RuntimeSemanticEvent::RunProgress { kind, text, .. } => vec![GatewayEvent::StatusText {
            text: text.unwrap_or(kind),
        }],
        RuntimeSemanticEvent::ToolCallFinished {
            tool_name,
            status,
            summary,
            error_message,
            ..
        } => {
            let summary = summary
                .or_else(|| error_message.clone())
                .unwrap_or_else(|| format!("Finished {tool_name}"));
            let mut events = vec![GatewayEvent::StatusText { text: summary }];
            if status == ToolCallFinishStatus::Error {
                if let Some(message) = error_message {
                    events.push(GatewayEvent::Error {
                        message,
                        detail: Some(format!("Tool `{tool_name}` returned an error.")),
                        error_type: Some("tool_execution_error".to_string()),
                        request_id: None,
                        context: None,
                    });
                }
            }
            events
        }
    }
}

pub fn runtime_stream_event_to_bearwire_sse(event: RuntimeStreamEvent) -> Vec<Bytes> {
    match event {
        RuntimeStreamEvent::Semantic(event) => runtime_semantic_event_to_bearwire_gateway_events(event)
            .into_iter()
            .map(gateway_event_to_adapter_sse)
            .collect(),
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_contracts::{RuntimeConversationRef, RuntimeErrorCategory};
    use serde_json::json;

    #[test]
    fn lossy_projection_handles_progress_events_without_failure() {
        let outcome = project_runtime_event_lossy(RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::RunProgress {
                kind: "plan_update".to_string(),
                text: None,
                phase: Some("tool_result".to_string()),
                detail: Some(json!({ "entries": [] })),
            },
        ));

        assert!(matches!(outcome, RuntimeEventProjectionOutcome::Events(events) if !events.is_empty()));
    }

    #[test]
    fn lossy_projection_covers_core_semantic_variants() {
        let events = vec![
            RuntimeSemanticEvent::AssistantTextDelta { text: "hi".to_string() },
            RuntimeSemanticEvent::StatusText { text: "working".to_string() },
            RuntimeSemanticEvent::ConversationResolved { conversation: RuntimeConversationRef { id: "conv".to_string() } },
            RuntimeSemanticEvent::TurnCompleted { turn: None },
            RuntimeSemanticEvent::RunPaused { reason: "awaiting_approval".to_string(), resume_token: None, expires_at: None },
            RuntimeSemanticEvent::Error { message: "err".to_string(), detail: None, error_type: None, request_id: None, context: None },
            RuntimeSemanticEvent::TurnFailed { category: RuntimeErrorCategory::Internal, message: "failed".to_string(), turn: None },
            RuntimeSemanticEvent::TurnCancelled { turn: None },
            RuntimeSemanticEvent::RunProgress { kind: "status_text".to_string(), text: Some("status".to_string()), phase: None, detail: None },
            RuntimeSemanticEvent::ToolCallFinished { tool_call_id: "call".to_string(), tool_name: "tool".to_string(), status: ToolCallFinishStatus::Ok, summary: Some("done".to_string()), error_message: None },
        ];

        for event in events {
            let outcome = project_runtime_event_lossy(RuntimeStreamEvent::Semantic(event));
            assert!(matches!(outcome, RuntimeEventProjectionOutcome::Events(_)));
        }
    }
}

#[cfg(test)]
mod test;

#[cfg(test)]
mod golden_traces_tests;
