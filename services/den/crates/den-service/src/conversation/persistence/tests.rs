    use super::*;
    use time::OffsetDateTime;

    fn row(
        message_type: &str,
        role: Option<&str>,
        visibility: &str,
    ) -> PersistedConversationMessage {
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
    fn user_history_projection_remains_text_only_when_tool_records_exist() {
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
    }
