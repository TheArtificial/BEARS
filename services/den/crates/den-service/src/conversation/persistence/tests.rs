use super::*;
use time::OffsetDateTime;

fn row(message_type: &str, role: Option<&str>, visibility: &str) -> PersistedConversationMessage {
    PersistedConversationMessage {
        sequence_no: 7,
        message_type: message_type.to_string(),
        role: role.map(str::to_string),
        visibility: visibility.to_string(),
        content_text: "hello transcript".to_string(),
        content_json: serde_json::json!({}),
        provider_message_id: Some("provider-7".to_string()),
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

#[test]
fn transcript_projection_accepts_canonical_and_legacy_message_shapes() {
    for message in [
        row("user", None, "default"),
        row("assistant", None, "default"),
        row("message", Some("user"), "default"),
        row("message", Some("assistant"), "default"),
    ] {
        assert!(message.to_model_transcript_message().is_some());
        assert!(message.to_user_history_transcript_message().is_some());
    }
}

#[test]
fn transcript_projection_separates_model_replay_from_user_history_visibility() {
    let hidden = row("user", Some("user"), "hidden_from_user");
    assert!(hidden.to_model_transcript_message().is_some());
    assert!(hidden.to_user_history_transcript_message().is_none());

    let diagnostic = row("assistant", Some("assistant"), "diagnostic_only");
    assert!(diagnostic.to_model_transcript_message().is_none());
    assert!(diagnostic.to_user_history_transcript_message().is_none());
}

#[test]
fn transcript_projection_rejects_non_transcript_roles() {
    let workflow = row("workflow_event", Some("system"), "default");
    assert!(workflow.to_model_transcript_message().is_none());
    assert!(workflow.to_user_history_transcript_message().is_none());
}

#[test]
fn transcript_projection_includes_tool_records_for_model_replay_only() {
    let tool_call = PersistedConversationMessage {
        sequence_no: 8,
        message_type: "tool_call".to_string(),
        role: Some("system".to_string()),
        visibility: "hidden_from_user".to_string(),
        content_text: "Tool request: memory_read".to_string(),
        content_json: serde_json::json!({
            "event": "tool_request",
            "tool_call_id": "call-1",
            "tool_name": "memory_read",
            "args": { "path": "pair/notes/demo.md" }
        }),
        provider_message_id: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
    };
    assert!(matches!(
        tool_call.to_model_transcript_record(),
        Some(PersistedTranscriptRecord::ToolCall { tool_call_id, tool_name, .. })
        if tool_call_id == "call-1" && tool_name == "memory_read"
    ));
    assert!(tool_call.to_user_history_transcript_message().is_none());

    let tool_result = PersistedConversationMessage {
        sequence_no: 9,
        message_type: "tool_result".to_string(),
        role: Some("system".to_string()),
        visibility: "hidden_from_user".to_string(),
        content_text: "Tool result: memory_read".to_string(),
        content_json: serde_json::json!({
            "event": "tool_result",
            "tool_call_id": "call-1",
            "tool_name": "memory_read",
            "status": "ok",
            "content": "file contents",
            "structured_content": { "content": "file contents" }
        }),
        provider_message_id: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
    };
    assert!(matches!(
        tool_result.to_model_transcript_record(),
        Some(PersistedTranscriptRecord::ToolResult { tool_call_id, status, .. })
        if tool_call_id.as_deref() == Some("call-1") && status.as_deref() == Some("ok")
    ));
    assert!(tool_result.to_user_history_transcript_message().is_none());
}

#[test]
fn typed_tool_payloads_decode_from_persisted_content_json() {
    let tool_call_json = serde_json::json!({
        "event": "tool_request",
        "tool_call_id": "call-1",
        "tool_name": "memory_read",
        "args": { "path": "pair/notes/demo.md" },
        "approval_required": false
    });
    let tool_call =
        PersistedToolRequestPayload::try_from(&tool_call_json).expect("tool call payload");
    assert_eq!(tool_call.tool_call_id, "call-1");
    assert_eq!(tool_call.tool_name, "memory_read");

    let tool_result_json = serde_json::json!({
        "event": "tool_result",
        "tool_call_id": "call-1",
        "tool_name": "memory_read",
        "status": "ok",
        "content": "file contents",
        "structured_content": { "content": "file contents" },
        "output_summary": "Used memory_read (ok): file contents",
        "output_preview": "file contents"
    });
    let tool_result =
        PersistedToolResultPayload::try_from(&tool_result_json).expect("tool result payload");
    assert_eq!(
        tool_result.status,
        den_core::tools::result_compaction::ToolResultStatus::Ok
    );
    assert_eq!(tool_result.output_preview.as_deref(), Some("file contents"));
}

#[test]
fn strict_typed_payloads_require_output_summary_for_complete_tool_results() {
    std::env::set_var("BEARS_STRICT_TYPED_PAYLOADS", "1");
    let tool_result_json = serde_json::json!({
        "event": "tool_result",
        "tool_call_id": "call-1",
        "tool_name": "memory_read",
        "status": "ok",
        "content": "file contents",
        "structured_content": { "content": "file contents" }
    });
    let err = PersistedToolResultPayload::try_from(&tool_result_json)
        .expect_err("strict mode should reject missing output_summary");
    std::env::remove_var("BEARS_STRICT_TYPED_PAYLOADS");

    assert!(err.to_string().contains("output_summary"));
}

#[test]
fn user_history_projection_includes_tool_result_summary_but_not_tool_request() {
    let user = row("user", Some("user"), "default");
    assert!(matches!(
        user.to_user_history_record(),
        Some(PersistedUserHistoryMessage { role, content, .. })
        if role == "user" && content == "hello transcript"
    ));

    let tool_call = PersistedConversationMessage {
        sequence_no: 8,
        message_type: "tool_call".to_string(),
        role: Some("system".to_string()),
        visibility: "diagnostic_only".to_string(),
        content_text: "Tool request: memory_read".to_string(),
        content_json: serde_json::json!({
            "event": "tool_request",
            "tool_call_id": "call-1",
            "tool_name": "memory_read",
            "args": { "path": "pair/notes/demo.md" }
        }),
        provider_message_id: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
    };
    assert!(tool_call.to_model_transcript_record().is_some());
    assert!(tool_call.to_user_history_record().is_none());

    let tool_result = PersistedConversationMessage {
        sequence_no: 9,
        message_type: "tool_result".to_string(),
        role: Some("system".to_string()),
        visibility: "diagnostic_only".to_string(),
        content_text: "Tool result: memory_read".to_string(),
        content_json: serde_json::json!({
            "event": "tool_result",
            "tool_call_id": "call-1",
            "tool_name": "memory_read",
            "status": "ok",
            "content": "file contents"
        }),
        provider_message_id: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
    };
    assert!(matches!(
        tool_result.to_user_history_record(),
        Some(PersistedUserHistoryMessage { role, content, .. })
        if role == "assistant" && content == "Used memory_read (ok): file contents"
    ));
}
