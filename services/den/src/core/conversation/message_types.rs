//! Typed conversation message storage values aligned with Postgres CHECK constraints
//! on `conversation_messages` ([`super::persistence`]).

use serde_json::Value;

use crate::errors::CustomError;

/// Allowed `conversation_messages.message_type` values (migration `20260530213000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConversationMessageType {
    User,
    Assistant,
    System,
    Developer,
    ToolCall,
    ToolResult,
    WorkflowEvent,
    CompactionMarker,
}

impl ConversationMessageType {
    pub const ALL: [Self; 8] = [
        Self::User,
        Self::Assistant,
        Self::System,
        Self::Developer,
        Self::ToolCall,
        Self::ToolResult,
        Self::WorkflowEvent,
        Self::CompactionMarker,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Developer => "developer",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::WorkflowEvent => "workflow_event",
            Self::CompactionMarker => "compaction_marker",
        }
    }

    pub fn try_from_storage(value: &str) -> Result<Self, CustomError> {
        match value.trim() {
            "user" | "user_input" => Ok(Self::User),
            "assistant" | "assistant_output" | "assistant_reasoning" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            "developer" => Ok(Self::Developer),
            "tool_call" => Ok(Self::ToolCall),
            "tool_result" | "tool_event" => Ok(Self::ToolResult),
            "workflow_event" | "prompt_memory_diagnostic" => Ok(Self::WorkflowEvent),
            "compaction_marker" => Ok(Self::CompactionMarker),
            other => Err(CustomError::ValidationError(format!(
                "unsupported conversation message_type: {other}"
            ))),
        }
    }
}

/// Allowed `conversation_messages.visibility` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConversationMessageVisibility {
    Default,
    HiddenFromUser,
    AdminOnly,
    DiagnosticOnly,
}

impl ConversationMessageVisibility {
    pub const ALL: [Self; 4] = [
        Self::Default,
        Self::HiddenFromUser,
        Self::AdminOnly,
        Self::DiagnosticOnly,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::HiddenFromUser => "hidden_from_user",
            Self::AdminOnly => "admin_only",
            Self::DiagnosticOnly => "diagnostic_only",
        }
    }

    pub fn try_from_storage(value: &str) -> Result<Self, CustomError> {
        match value.trim() {
            "default" | "visible" => Ok(Self::Default),
            "hidden_from_user" => Ok(Self::HiddenFromUser),
            "admin_only" => Ok(Self::AdminOnly),
            "diagnostic_only" => Ok(Self::DiagnosticOnly),
            other => Err(CustomError::ValidationError(format!(
                "unsupported conversation message visibility: {other}"
            ))),
        }
    }

    pub fn is_transcript_visible(self) -> bool {
        matches!(self, Self::Default | Self::HiddenFromUser)
    }
}

/// Roles stored on `conversation_messages.role` when present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConversationMessageRole {
    User,
    Assistant,
    System,
    Developer,
}

impl ConversationMessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Developer => "developer",
        }
    }

    pub fn try_from_storage(value: &str) -> Result<Self, CustomError> {
        match value.trim() {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            "developer" => Ok(Self::Developer),
            other => Err(CustomError::ValidationError(format!(
                "unsupported conversation message role: {other}"
            ))),
        }
    }
}

/// Typed write payload for [`super::persistence::append_message`].
#[derive(Debug, Clone)]
pub struct ConversationMessageWrite {
    pub message_type: ConversationMessageType,
    pub role: Option<ConversationMessageRole>,
    pub visibility: ConversationMessageVisibility,
    pub content_text: String,
    pub content_json: Value,
    pub provider_message_id: Option<String>,
    pub source_event_id: Option<String>,
    pub created_at: Option<String>,
}

impl ConversationMessageWrite {
    pub fn structured(
        message_type: ConversationMessageType,
        role: Option<ConversationMessageRole>,
        visibility: ConversationMessageVisibility,
        content_text: impl Into<String>,
        content_json: Value,
    ) -> Self {
        Self {
            message_type,
            role,
            visibility,
            content_text: content_text.into(),
            content_json,
            provider_message_id: None,
            source_event_id: None,
            created_at: None,
        }
    }

    pub fn user_turn(
        content_text: impl Into<String>,
        content_json: Value,
        source_event_id: Option<String>,
    ) -> Self {
        Self {
            message_type: ConversationMessageType::User,
            role: Some(ConversationMessageRole::User),
            visibility: ConversationMessageVisibility::Default,
            content_text: content_text.into(),
            content_json,
            provider_message_id: None,
            source_event_id,
            created_at: None,
        }
    }

    pub fn assistant_turn(content_text: impl Into<String>, content_json: Value) -> Self {
        Self {
            message_type: ConversationMessageType::Assistant,
            role: Some(ConversationMessageRole::Assistant),
            visibility: ConversationMessageVisibility::Default,
            content_text: content_text.into(),
            content_json,
            provider_message_id: None,
            source_event_id: None,
            created_at: None,
        }
    }

    pub fn assistant_reasoning_diagnostic(
        content_text: impl Into<String>,
        content_json: Value,
    ) -> Self {
        Self::structured(
            ConversationMessageType::WorkflowEvent,
            Some(ConversationMessageRole::Assistant),
            ConversationMessageVisibility::DiagnosticOnly,
            content_text,
            content_json,
        )
    }

    pub fn workflow_diagnostic(content_text: impl Into<String>, content_json: Value) -> Self {
        Self::structured(
            ConversationMessageType::WorkflowEvent,
            None,
            ConversationMessageVisibility::DiagnosticOnly,
            content_text,
            content_json,
        )
    }

    pub fn with_provider_message_id(mut self, provider_message_id: Option<String>) -> Self {
        self.provider_message_id = provider_message_id;
        self
    }

    pub fn with_source_event_id(mut self, source_event_id: Option<String>) -> Self {
        self.source_event_id = source_event_id;
        self
    }

    pub fn with_created_at(mut self, created_at: Option<String>) -> Self {
        self.created_at = created_at;
        self
    }
}

#[cfg(test)]
mod tests {
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
}
