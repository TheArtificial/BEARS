use crate::core::{
    acp_letta_events::AcpGatewayEvent,
    runtime_bearwire_projection::{
        runtime_semantic_event_to_bearwire_gateway_event, runtime_stream_event_to_bearwire_sse,
    },
    runtime_contracts::{RuntimeConversationRef, RuntimeSemanticEvent, RuntimeStreamEvent},
};

#[test]
fn semantic_assistant_text_projects_to_bearwire_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_event(
        RuntimeSemanticEvent::AssistantTextDelta {
            text: "hello".to_string(),
        },
    )
    .expect("projection should succeed");

    assert!(matches!(
        mapped,
        AcpGatewayEvent::AssistantTextDelta { text } if text == "hello"
    ));
}

#[test]
fn semantic_turn_completed_projects_to_bearwire_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_event(
        RuntimeSemanticEvent::TurnCompleted { turn: None },
    )
    .expect("projection should succeed");

    assert!(matches!(
        mapped,
        AcpGatewayEvent::TurnComplete { outcome } if outcome == "ok"
    ));
}

#[test]
fn untranslated_provider_event_does_not_project_to_bearwire_sse() {
    let mapped = runtime_stream_event_to_bearwire_sse(
        RuntimeStreamEvent::UntranslatedProviderEvent {
            value: serde_json::json!({"message_type":"tool_return_message"}),
        },
    );
    assert!(mapped.is_none());
}

#[test]
fn semantic_conversation_resolved_projects_to_bearwire_sse() {
    let bytes = runtime_stream_event_to_bearwire_sse(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::ConversationResolved {
            conversation: RuntimeConversationRef {
                id: "conv-test".to_string(),
            },
        },
    ))
    .expect("projection should succeed");
    let text = String::from_utf8(bytes.to_vec()).expect("utf8 sse");
    assert!(text.contains("conversation_resolved"));
    assert!(text.contains("conv-test"));
}
