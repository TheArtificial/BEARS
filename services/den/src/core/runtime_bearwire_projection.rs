use bytes::Bytes;

use crate::core::{
    acp_letta_events::{acp_event_to_adapter_sse, AcpGatewayEvent},
    runtime_contracts::{RuntimeSemanticEvent, RuntimeStreamEvent},
};

pub fn runtime_semantic_event_to_bearwire_gateway_event(
    event: RuntimeSemanticEvent,
) -> Option<AcpGatewayEvent> {
    match event {
        RuntimeSemanticEvent::AssistantTextDelta { text } => {
            Some(AcpGatewayEvent::AssistantTextDelta { text })
        }
        RuntimeSemanticEvent::StatusText { text } => Some(AcpGatewayEvent::StatusText { text }),
        RuntimeSemanticEvent::ConversationResolved { conversation } => {
            Some(AcpGatewayEvent::ConversationResolved {
                conversation_id: conversation.id,
            })
        }
        RuntimeSemanticEvent::TurnCompleted { .. } => Some(AcpGatewayEvent::TurnComplete {
            outcome: "ok".to_string(),
        }),
        RuntimeSemanticEvent::Error {
            message,
            detail,
            error_type,
            request_id,
            context,
        } => Some(AcpGatewayEvent::Error {
            message,
            detail,
            error_type,
            request_id,
            context,
        }),
        RuntimeSemanticEvent::RunPaused { .. }
        | RuntimeSemanticEvent::RunProgress { .. }
        | RuntimeSemanticEvent::ToolCallRequested { .. }
        | RuntimeSemanticEvent::TurnFailed { .. }
        | RuntimeSemanticEvent::TurnCancelled { .. } => None,
    }
}

pub fn runtime_stream_event_to_bearwire_sse(event: RuntimeStreamEvent) -> Option<Bytes> {
    match event {
        RuntimeStreamEvent::Semantic(event) => {
            runtime_semantic_event_to_bearwire_gateway_event(event).map(acp_event_to_adapter_sse)
        }
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => None,
    }
}
