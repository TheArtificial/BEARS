use crate::core::runtime_provider::{RuntimeSemanticEvent, RuntimeStreamEvent};

#[test]
fn semantic_turn_completed_uses_direct_gateway_projection_shape() {
    let projected = crate::core::runtime_bearwire_projection::runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::TurnCompleted { turn: None },
    );
    assert!(matches!(
        projected.as_slice(),
        [crate::core::acp_events::AcpGatewayEvent::TurnComplete { outcome }]
            if outcome == "ok"
    ));
}

#[test]
fn semantic_run_paused_uses_direct_gateway_projection_shape() {
    let projected = crate::core::runtime_bearwire_projection::runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::RunPaused {
            reason: "awaiting_approval".to_string(),
            resume_token: None,
            expires_at: None,
        },
    );
    assert!(matches!(
        projected.as_slice(),
        [crate::core::acp_events::AcpGatewayEvent::StatusText { text }]
            if text == "Waiting for approval."
    ));
}

#[test]
fn untranslated_provider_event_still_requires_seed_path() {
    let projected = crate::core::runtime_bearwire_projection::runtime_stream_event_to_bearwire_sse(
        RuntimeStreamEvent::UntranslatedProviderEvent {
            value: serde_json::json!({"message_type":"tool_return_message"}),
        },
    );
    assert!(projected.is_empty());
}
