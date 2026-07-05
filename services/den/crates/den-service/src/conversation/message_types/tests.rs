use super::*;

#[test]
fn message_type_values_match_postgres_check_constraint() {
    let allowed = [
        "user",
        "assistant",
        "system",
        "developer",
        "tool_call",
        "tool_result",
        "workflow_event",
        "compaction_marker",
    ];
    for ty in ConversationMessageType::ALL {
        assert!(allowed.contains(&ty.as_str()));
    }
}

#[test]
fn visibility_values_match_postgres_check_constraint() {
    let allowed = [
        "default",
        "hidden_from_user",
        "admin_only",
        "diagnostic_only",
    ];
    for visibility in ConversationMessageVisibility::ALL {
        assert!(allowed.contains(&visibility.as_str()));
    }
}

#[test]
fn user_turn_builder_uses_schema_valid_storage_values() {
    let write = ConversationMessageWrite::user_turn(
        "hello",
        serde_json::json!({"type":"user_input"}),
        Some("web-chat-user-input:1".to_string()),
    );
    assert_eq!(write.message_type, ConversationMessageType::User);
    assert_eq!(write.visibility, ConversationMessageVisibility::Default);
    assert_eq!(write.role, Some(ConversationMessageRole::User));
}
