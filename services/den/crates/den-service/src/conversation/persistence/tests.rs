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
