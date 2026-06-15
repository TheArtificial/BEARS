use crate::acp::stream::support::{
    classify_untranslated_provider_event, AcpStreamDiagnostics,
};

#[test]
fn untranslated_classifier_prefers_message_type() {
    let class = classify_untranslated_provider_event(&serde_json::json!({
        "message_type": "tool_return_message",
        "type": "ignored"
    }));
    assert_eq!(class, "message_type:tool_return_message");
}

#[test]
fn untranslated_classifier_falls_back_to_type() {
    let class = classify_untranslated_provider_event(&serde_json::json!({
        "type": "conversation_resolved"
    }));
    assert_eq!(class, "type:conversation_resolved");
}

#[test]
fn untranslated_observation_records_class_counts() {
    let mut diagnostics = AcpStreamDiagnostics::default();
    diagnostics.observe_unmapped_event(&serde_json::json!({
        "message_type": "tool_return_message",
        "content": "ok"
    }));
    diagnostics.observe_unmapped_event(&serde_json::json!({
        "message_type": "tool_return_message",
        "content": "ok-again"
    }));

    assert_eq!(diagnostics.unmapped_events, 2);
    assert_eq!(
        diagnostics
            .untranslated_event_classes
            .get("message_type:tool_return_message")
            .copied(),
        Some(2)
    );
}
