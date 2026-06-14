use bytes::Bytes;

use crate::core::{
    acp_events::{acp_event_to_adapter_sse, AcpGatewayEvent},
    runtime_contracts::{
        RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent, ToolCallFinishStatus,
    },
};

pub fn runtime_semantic_event_to_bearwire_gateway_events(
    event: RuntimeSemanticEvent,
) -> Vec<AcpGatewayEvent> {
    match event {
        RuntimeSemanticEvent::AssistantTextDelta { text } => {
            vec![AcpGatewayEvent::AssistantTextDelta { text }]
        }
        RuntimeSemanticEvent::StatusText { text } => vec![AcpGatewayEvent::StatusText { text }],
        RuntimeSemanticEvent::ConversationResolved { conversation } => {
            vec![AcpGatewayEvent::ConversationResolved {
                conversation_id: conversation.id,
            }]
        }
        RuntimeSemanticEvent::TurnCompleted { .. } => vec![AcpGatewayEvent::TurnComplete {
            outcome: "ok".to_string(),
        }],
        RuntimeSemanticEvent::RunPaused { reason, .. } => vec![AcpGatewayEvent::StatusText {
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
        } => vec![AcpGatewayEvent::ToolRequest {
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
        } => vec![AcpGatewayEvent::Error {
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
        } => vec![AcpGatewayEvent::Error {
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
        RuntimeSemanticEvent::TurnCancelled { .. } => vec![AcpGatewayEvent::Error {
            message: "Runtime continuation was cancelled.".to_string(),
            detail: None,
            error_type: Some("runtime_turn_cancelled".to_string()),
            request_id: None,
            context: None,
        }],
        RuntimeSemanticEvent::RunProgress { kind, text, .. } => vec![AcpGatewayEvent::StatusText {
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
                .or(error_message.clone())
                .unwrap_or_else(|| format!("Finished {tool_name}"));
            let mut events = vec![AcpGatewayEvent::StatusText { text: summary }];
            if status == ToolCallFinishStatus::Error {
                if let Some(message) = error_message {
                    events.push(AcpGatewayEvent::Error {
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
            .map(acp_event_to_adapter_sse)
            .collect(),
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod golden_traces_tests;
#[cfg(test)]
mod test;
