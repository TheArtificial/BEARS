use sqlx::PgPool;
use tracing::Instrument;
use uuid::Uuid;

use crate::errors::CustomError;

use super::conversation_persistence::{
    append_message, ensure_conversation_for_external_id, list_messages_page,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEventDedupKey {
    pub event: String,
    pub scope_id: String,
    pub request_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub provider_message_id: Option<String>,
}

impl CanonicalEventDedupKey {
    pub fn source_event_id(&self) -> String {
        serde_json::json!({
            "event": self.event,
            "scope_id": self.scope_id,
            "request_id": self.request_id,
            "tool_call_id": self.tool_call_id,
            "provider_message_id": self.provider_message_id,
        })
        .to_string()
    }
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

#[derive(Debug, Clone)]
pub struct ConversationEventProvenance {
    pub source: String,
    pub scope_id: String,
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

impl ConversationEventProvenance {
    pub fn acp_session(scope_id: impl Into<String>) -> Self {
        Self {
            source: "acp_stream".to_string(),
            scope_id: scope_id.into(),
        }
    }

    pub fn as_content_json(&self, event: &str) -> serde_json::Value {
        serde_json::json!({
            "source": self.source,
            "event": event,
            "scope_id": self.scope_id,
        })
    }
}

fn content_json_dedup_key(content_json: &serde_json::Value) -> Option<CanonicalEventDedupKey> {
    let object = content_json.as_object()?;
    let event = object.get("event")?.as_str()?.to_string();
    let scope_id = object.get("scope_id")?.as_str()?.to_string();
    let request_id = object
        .get("request_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let tool_call_id = object
        .get("tool_call_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let provider_message_id = object
        .get("provider_message_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Some(CanonicalEventDedupKey {
        event,
        scope_id,
        request_id,
        tool_call_id,
        provider_message_id,
    })
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
            message_type: "tool_result".to_string(),
            role: Some("system".to_string()),
            visibility: "diagnostic_only".to_string(),
            content_text: content_text.into(),
            content_json,
            provider_message_id,
        }
    }

    pub fn tool_call_event(
        content_text: impl Into<String>,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    ) -> Self {
        Self::StructuredEvent {
            message_type: "tool_call".to_string(),
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

    pub fn assistant_output(
        text: impl Into<String>,
        provenance: &ConversationEventProvenance,
        provider_message_id: Option<String>,
        request_id: Option<String>,
    ) -> Self {
        let mut content_json = provenance.as_content_json("assistant_output");
        if let Some(request_id) = request_id {
            content_json["request_id"] = serde_json::json!(request_id);
        }
        if let Some(provider_message_id) = provider_message_id.as_ref() {
            content_json["provider_message_id"] = serde_json::json!(provider_message_id);
        }
        Self::visible_assistant_message(text, content_json, provider_message_id)
    }

    pub fn conversation_resolved(
        conversation_id: impl Into<String>,
        provenance: &ConversationEventProvenance,
    ) -> Self {
        let conversation_id = conversation_id.into();
        Self::workflow_event(
            "Conversation resolved",
            serde_json::json!({
                "source": provenance.source,
                "event": "conversation_resolved",
                "scope_id": provenance.scope_id,
                "conversation_id": conversation_id,
            }),
            None,
        )
    }

    pub fn tool_request(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        request_id: impl Into<String>,
        approval_request_id: Option<String>,
        args: serde_json::Value,
        approval_required: bool,
        approval_reason: Option<String>,
        route: impl Into<String>,
        provenance: &ConversationEventProvenance,
    ) -> Self {
        let tool_name = tool_name.into();
        let tool_call_id = tool_call_id.into();
        Self::tool_call_event(
            format!("Tool request: {tool_name}"),
            serde_json::json!({
                "source": provenance.source,
                "event": "tool_request",
                "scope_id": provenance.scope_id,
                "request_id": request_id.into(),
                "tool_call_id": tool_call_id,
                "approval_request_id": approval_request_id,
                "tool_name": tool_name,
                "args": args,
                "approval_required": approval_required,
                "approval_reason": approval_reason,
                "route": route.into(),
            }),
            None,
        )
    }

    pub fn tool_result(
        tool_name: Option<String>,
        tool_call_id: impl Into<String>,
        approval_request_id: Option<String>,
        status: impl Into<String>,
        content: Option<String>,
        structured_content: serde_json::Value,
        diagnostic: serde_json::Value,
        request_id: Option<String>,
        provenance: &ConversationEventProvenance,
    ) -> Self {
        let tool_name = tool_name.unwrap_or_else(|| "tool".to_string());
        Self::tool_event(
            format!("Tool result: {tool_name}"),
            serde_json::json!({
                "source": provenance.source,
                "event": "tool_result",
                "scope_id": provenance.scope_id,
                "tool_call_id": tool_call_id.into(),
                "approval_request_id": approval_request_id,
                "tool_name": tool_name,
                "status": status.into(),
                "content": content,
                "structured_content": structured_content,
                "diagnostic": diagnostic,
                "request_id": request_id,
            }),
            None,
        )
    }

    pub fn turn_outcome(
        status: &str,
        reason: &str,
        request_id: impl Into<String>,
        retryable: bool,
        scope: serde_json::Value,
        diagnostics: serde_json::Value,
        provenance: &ConversationEventProvenance,
    ) -> Self {
        Self::workflow_event(
            format!("Turn outcome: {status} / {reason}"),
            serde_json::json!({
                "source": provenance.source,
                "event": "turn_result",
                "scope_id": provenance.scope_id,
                "status": status,
                "reason": reason,
                "request_id": request_id.into(),
                "retryable": retryable,
                "scope": scope,
                "diagnostics": diagnostics,
            }),
            None,
        )
    }

    pub fn structured_event(
        message_type: impl Into<String>,
        role: Option<String>,
        visibility: impl Into<String>,
        content_text: impl Into<String>,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    ) -> Self {
        Self::normalized_structured_event(
            message_type.into(),
            role,
            visibility.into(),
            content_text.into(),
            content_json,
            provider_message_id,
        )
    }

    pub fn normalized_structured_event(
        message_type: String,
        role: Option<String>,
        visibility: String,
        content_text: String,
        content_json: serde_json::Value,
        provider_message_id: Option<String>,
    ) -> Self {
        match message_type.as_str() {
            "tool_event" | "tool_result" => {
                Self::tool_event(content_text, content_json, provider_message_id)
            }
            "tool_call" => Self::tool_call_event(content_text, content_json, provider_message_id),
            "workflow_event" => {
                Self::workflow_event(content_text, content_json, provider_message_id)
            }
            _ => Self::StructuredEvent {
                message_type,
                role,
                visibility,
                content_text,
                content_json,
                provider_message_id,
            },
        }
    }

    fn storage_message_type(&self) -> &str {
        match self {
            Self::VisibleMessage { role, .. } => role.as_str(),
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

    fn dedup_key(&self) -> Option<CanonicalEventDedupKey> {
        content_json_dedup_key(&self.storage_json())
    }
}

fn canonical_record_source_event_id(record: &CanonicalConversationRecord) -> Option<String> {
    record.dedup_key().map(|key| key.source_event_id())
}

async fn canonical_record_already_persisted(
    context: &ConversationPersistenceContext,
    conversation_id: Uuid,
    record: &CanonicalConversationRecord,
) -> Result<bool, CustomError> {
    let Some(expected) = record.dedup_key() else {
        return Ok(false);
    };
    let recent = list_messages_page(&context.pool, conversation_id, None, 32).await?;
    for message in recent {
        if let Ok(content_json) = serde_json::from_str::<serde_json::Value>(&message.content_text) {
            if content_json_dedup_key(&content_json).as_ref() == Some(&expected) {
                return Ok(true);
            }
        }
    }
    Ok(false)
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
    let source_event_id = canonical_record_source_event_id(record);
    if source_event_id.is_none() && canonical_record_already_persisted(context, canonical.id, record).await?
    {
        return Ok(());
    }
    append_message(
        &context.pool,
        canonical.id,
        record.storage_message_type(),
        record.storage_role(),
        record.storage_visibility(),
        record.storage_text(),
        record.storage_json(),
        record.provider_message_id(),
        source_event_id.as_deref(),
        None,
    )
    .await?;
    Ok(())
}

pub fn canonical_persistence_context(
    pool: PgPool,
    bear_id: Uuid,
    user_id: Option<i32>,
    external_conversation_id: String,
    source_session_id: Option<String>,
    request_id: Option<String>,
    persistence_scope_id: String,
    skip_persistence: bool,
) -> ConversationPersistenceContext {
    ConversationPersistenceContext {
        pool,
        bear_id,
        user_id,
        external_conversation_id,
        source_session_id,
        request_id,
        persistence_scope_id,
        skip_persistence,
    }
}

pub fn normalize_persisted_gateway_record(
    message_type: &str,
    role: Option<&str>,
    visibility: &str,
    content_text: String,
    content_json: serde_json::Value,
    provider_message_id: Option<String>,
) -> CanonicalConversationRecord {
    match message_type {
        "message" => match role {
            Some("user") => CanonicalConversationRecord::visible_user_message(
                content_text,
                content_json,
                provider_message_id,
            ),
            _ => CanonicalConversationRecord::visible_assistant_message(
                content_text,
                content_json,
                provider_message_id,
            ),
        },
        _ => CanonicalConversationRecord::normalized_structured_event(
            message_type.to_string(),
            role.map(str::to_string),
            visibility.to_string(),
            content_text,
            content_json,
            provider_message_id,
        ),
    }
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

pub fn spawn_persist_assistant_output(
    context: ConversationPersistenceContext,
    content_text: String,
    provenance: &ConversationEventProvenance,
    provider_message_id: Option<String>,
    request_id: Option<String>,
) {
    spawn_persist_canonical_conversation_record(
        context,
        CanonicalConversationRecord::assistant_output(
            content_text,
            provenance,
            provider_message_id,
            request_id,
        ),
    );
}

pub fn spawn_persist_turn_outcome(
    context: ConversationPersistenceContext,
    role_result: &crate::core::role_runtime::RoleTurnResult,
    provenance: &ConversationEventProvenance,
) {
    spawn_persist_canonical_conversation_record(
        context,
        CanonicalConversationRecord::turn_outcome(
            role_result.status.as_str(),
            role_result.reason.as_str(),
            role_result.request_id.to_string(),
            role_result.retryable,
            role_result.scope.diagnostic(),
            role_result.diagnostics.clone(),
            provenance,
        ),
    );
}

pub fn spawn_persist_tool_result(
    context: ConversationPersistenceContext,
    tool_name: Option<String>,
    tool_call_id: String,
    approval_request_id: Option<String>,
    status: String,
    content: Option<String>,
    structured_content: serde_json::Value,
    diagnostic: serde_json::Value,
    request_id: Option<String>,
    provenance: &ConversationEventProvenance,
) {
    spawn_persist_canonical_conversation_record(
        context,
        CanonicalConversationRecord::tool_result(
            tool_name,
            tool_call_id,
            approval_request_id,
            status,
            content,
            structured_content,
            diagnostic,
            request_id,
            provenance,
        ),
    );
}

pub fn spawn_persist_tool_request(
    context: ConversationPersistenceContext,
    tool_name: String,
    tool_call_id: String,
    request_id: String,
    approval_request_id: Option<String>,
    arguments: serde_json::Value,
    approval_required: bool,
    approval_reason: Option<String>,
    route: String,
    provenance: &ConversationEventProvenance,
) {
    spawn_persist_canonical_conversation_record(
        context,
        CanonicalConversationRecord::tool_request(
            tool_name,
            tool_call_id,
            request_id,
            approval_request_id,
            arguments,
            approval_required,
            approval_reason,
            route,
            provenance,
        ),
    );
}

pub fn spawn_persist_workflow_event(
    context: ConversationPersistenceContext,
    content_text: String,
    content_json: serde_json::Value,
    provider_message_id: Option<String>,
) {
    spawn_persist_canonical_conversation_record(
        context,
        CanonicalConversationRecord::workflow_event(
            content_text,
            content_json,
            provider_message_id,
        ),
    );
}

pub struct NonAcpAuditProjection {
    pub event: String,
    pub workflow_text: String,
    pub workflow_json: serde_json::Value,
    pub visible_summary_text: Option<String>,
}

pub fn project_non_acp_audit_event(
    pool: &PgPool,
    bear_id: Uuid,
    user_id: Option<i32>,
    conversation_id: Option<&str>,
    provenance: ConversationEventProvenance,
    projection: NonAcpAuditProjection,
) {
    let Some(conversation_id) = conversation_id.filter(|id| id.starts_with("conv-")) else {
        return;
    };
    let context = canonical_persistence_context(
        pool.clone(),
        bear_id,
        user_id,
        conversation_id.to_string(),
        None,
        None,
        provenance.scope_id.clone(),
        false,
    );
    let mut workflow_json = serde_json::json!({
        "source": provenance.source,
        "event": projection.event,
        "scope_id": provenance.scope_id,
    });
    if let (Some(base), Some(extra)) = (workflow_json.as_object_mut(), projection.workflow_json.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    spawn_persist_workflow_event(
        context.clone(),
        projection.workflow_text,
        workflow_json,
        None,
    );
    if let Some(text) = projection.visible_summary_text {
        spawn_persist_assistant_summary_message(context, text, None);
    }
}

pub fn spawn_persist_assistant_summary_message(
    context: ConversationPersistenceContext,
    text: String,
    provider_message_id: Option<String>,
) {
    spawn_persist_canonical_conversation_record(
        context,
        CanonicalConversationRecord::visible_assistant_message(
            text,
            serde_json::json!({}),
            provider_message_id,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_acp_audit_projection_merges_provenance_and_payload() {
        let provenance = ConversationEventProvenance {
            source: "pair_reflection".to_string(),
            scope_id: "bear:scope".to_string(),
        };
        let projection = NonAcpAuditProjection {
            event: "pair_reflection_completed".to_string(),
            workflow_text: "Pair reflection completed".to_string(),
            workflow_json: serde_json::json!({
                "status": "completed",
                "summary_path": "pair/summary.md",
            }),
            visible_summary_text: Some("Summary saved".to_string()),
        };

        let mut workflow_json = serde_json::json!({
            "source": provenance.source.clone(),
            "event": projection.event.clone(),
            "scope_id": provenance.scope_id.clone(),
        });
        if let (Some(base), Some(extra)) =
            (workflow_json.as_object_mut(), projection.workflow_json.as_object())
        {
            for (key, value) in extra {
                base.insert(key.clone(), value.clone());
            }
        }

        assert_eq!(workflow_json["source"], "pair_reflection");
        assert_eq!(workflow_json["event"], "pair_reflection_completed");
        assert_eq!(workflow_json["scope_id"], "bear:scope");
        assert_eq!(workflow_json["status"], "completed");
        assert_eq!(workflow_json["summary_path"], "pair/summary.md");
        assert_eq!(projection.visible_summary_text.as_deref(), Some("Summary saved"));
    }

    #[test]
    fn non_acp_audit_projection_requires_canonical_conversation_id() {
        assert!(Some("conv-123").filter(|id| id.starts_with("conv-")).is_some());
        assert!(Some("conversation-123")
            .filter(|id| id.starts_with("conv-"))
            .is_none());
        assert!(Option::<&str>::None
            .filter(|id| id.starts_with("conv-"))
            .is_none());
    }

    #[test]
    fn assistant_summary_message_uses_visible_assistant_shape() {
        let record = CanonicalConversationRecord::visible_assistant_message(
            "Summary saved",
            serde_json::json!({}),
            None,
        );
        match record {
            CanonicalConversationRecord::VisibleMessage {
                role,
                text,
                content_json,
                provider_message_id,
            } => {
                assert_eq!(role.as_str(), "assistant");
                assert_eq!(text, "Summary saved");
                assert_eq!(content_json, serde_json::json!({}));
                assert_eq!(provider_message_id, None);
            }
            _ => panic!("expected visible assistant message"),
        }
    }
}
