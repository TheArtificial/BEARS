use sqlx::PgPool;
use tracing::Instrument;
use uuid::Uuid;

use crate::errors::CustomError;

use super::conversation_persistence::{append_message, ensure_conversation_for_external_id};

#[derive(Debug, Clone)]
pub struct ConversationPersistenceContext {
    pub pool: PgPool,
    pub bear_id: Uuid,
    pub user_id: Option<i32>,
    pub external_conversation_id: String,
    pub source_session_id: Option<String>,
    pub request_id: Option<String>,
    pub persistence_scope_id: String,
    pub skip_persistence: bool,
}

#[derive(Debug, Clone)]
pub enum CanonicalConversationRecord {
    VisibleMessage {
        role: CanonicalVisibleRole,
        text: String,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    },
    StructuredEvent {
        message_type: String,
        role: Option<String>,
        visibility: String,
        content_text: String,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum CanonicalVisibleRole {
    User,
    Assistant,
}

impl CanonicalVisibleRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

impl CanonicalConversationRecord {
    pub fn visible_user_message(
        text: impl Into<String>,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    ) -> Self {
        Self::VisibleMessage {
            role: CanonicalVisibleRole::User,
            text: text.into(),
            content_json,
            provider_message_id,
        }
    }

    pub fn visible_assistant_message(
        text: impl Into<String>,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    ) -> Self {
        Self::VisibleMessage {
            role: CanonicalVisibleRole::Assistant,
            text: text.into(),
            content_json,
            provider_message_id,
        }
    }

    pub fn tool_event(
        content_text: impl Into<String>,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    ) -> Self {
        Self::StructuredEvent {
            message_type: "tool_event".to_string(),
            role: Some("system".to_string()),
            visibility: "diagnostic_only".to_string(),
            content_text: content_text.into(),
            content_json,
            provider_message_id,
        }
    }

    pub fn workflow_event(
        content_text: impl Into<String>,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    ) -> Self {
        Self::StructuredEvent {
            message_type: "workflow_event".to_string(),
            role: Some("system".to_string()),
            visibility: "diagnostic_only".to_string(),
            content_text: content_text.into(),
            content_json,
            provider_message_id,
        }
    }

    pub fn structured_event(
        message_type: impl Into<String>,
        role: Option<String>,
        visibility: impl Into<String>,
        content_text: impl Into<String>,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    ) -> Self {
        Self::StructuredEvent {
            message_type: message_type.into(),
            role,
            visibility: visibility.into(),
            content_text: content_text.into(),
            content_json,
            provider_message_id,
        }
    }

    fn storage_message_type(&self) -> &str {
        match self {
            Self::VisibleMessage { .. } => "message",
            Self::StructuredEvent { message_type, .. } => message_type.as_str(),
        }
    }

    fn storage_role(&self) -> Option<&str> {
        match self {
            Self::VisibleMessage { role, .. } => Some(role.as_str()),
            Self::StructuredEvent { role, .. } => role.as_deref(),
        }
    }

    fn storage_visibility(&self) -> &str {
        match self {
            Self::VisibleMessage { .. } => "default",
            Self::StructuredEvent { visibility, .. } => visibility.as_str(),
        }
    }

    fn storage_text(&self) -> &str {
        match self {
            Self::VisibleMessage { text, .. } => text.as_str(),
            Self::StructuredEvent { content_text, .. } => content_text.as_str(),
        }
    }

    fn storage_json(&self) -> serde_json::Value {
        match self {
            Self::VisibleMessage { content_json, .. } => content_json.clone(),
            Self::StructuredEvent { content_json, .. } => content_json.clone(),
        }
    }

    fn provider_message_id(&self) -> Option<&str> {
        match self {
            Self::VisibleMessage {
                provider_message_id,
                ..
            }
            | Self::StructuredEvent {
                provider_message_id,
                ..
            } => provider_message_id.as_deref(),
        }
    }
}

pub async fn persist_canonical_conversation_record(
    context: &ConversationPersistenceContext,
    record: &CanonicalConversationRecord,
) -> Result<(), CustomError> {
    if context.skip_persistence {
        return Ok(());
    }
    if context.external_conversation_id != "default"
        && !context.external_conversation_id.starts_with("conv-")
    {
        return Ok(());
    }
    let canonical = ensure_conversation_for_external_id(
        &context.pool,
        context.bear_id,
        context.user_id,
        &context.external_conversation_id,
        context.source_session_id.as_deref(),
        None,
    )
    .await?;
    append_message(
        &context.pool,
        canonical.id,
        record.storage_message_type(),
        record.storage_role(),
        record.storage_visibility(),
        record.storage_text(),
        record.storage_json(),
        record.provider_message_id(),
        None,
    )
    .await?;
    Ok(())
}

pub fn spawn_persist_canonical_conversation_record(
    context: ConversationPersistenceContext,
    record: CanonicalConversationRecord,
) {
    if context.skip_persistence {
        return;
    }
    let span_request_id = context.request_id.clone();
    let span_scope = context.persistence_scope_id.clone();
    tokio::spawn(
        async move {
            if let Err(err) = persist_canonical_conversation_record(&context, &record).await {
                tracing::warn!(
                    request_id = ?context.request_id,
                    persistence_scope_id = %context.persistence_scope_id,
                    error = %err,
                    "canonical conversation record persistence failed"
                );
            }
        }
        .instrument(tracing::info_span!(
            "persist_canonical_conversation_record",
            request_id = ?span_request_id,
            persistence_scope_id = %span_scope,
        )),
    );
}
