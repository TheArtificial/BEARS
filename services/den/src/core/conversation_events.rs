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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionSource {
    AcpStream,
    MemoryProposals,
    PairReflection,
    ReflectionConductor,
    DenTools,
}

impl ProjectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AcpStream => "acp_stream",
            Self::MemoryProposals => "memory_proposals",
            Self::PairReflection => "pair_reflection",
            Self::ReflectionConductor => "reflection_conductor",
            Self::DenTools => "den_tools",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionEventKind {
    MemoryProposalCreated,
    MemoryProposalResolved,
    PairReflectionCompleted,
    MemoryCurateEnqueued,
    MemoryCurateStarted,
    MemoryCurateCompleted,
    MemoryCurateFailed,
    MemoryReviewRequested,
}

impl ProjectionEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MemoryProposalCreated => "memory_proposal_created",
            Self::MemoryProposalResolved => "memory_proposal_resolved",
            Self::PairReflectionCompleted => "pair_reflection_completed",
            Self::MemoryCurateEnqueued => "memory_curate_enqueued",
            Self::MemoryCurateStarted => "memory_curate_started",
            Self::MemoryCurateCompleted => "memory_curate_completed",
            Self::MemoryCurateFailed => "memory_curate_failed",
            Self::MemoryReviewRequested => "memory_review_requested",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectionProvenance {
    pub source: ProjectionSource,
    pub scope_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryProposalCreatedPayload {
    pub proposal_id: Uuid,
    pub source_role: String,
    pub suggested_action: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryProposalResolvedPayload {
    pub proposal_id: Uuid,
    pub source_role: String,
    pub suggested_action: String,
    pub title: String,
    pub status: String,
    pub reviewer_role: Option<String>,
    pub result_path: Option<String>,
    pub result_commit: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PairReflectionCompletedPayload {
    pub reflection_run_id: Uuid,
    pub acp_session_id: String,
    pub trigger: String,
    pub status: String,
    pub summary_path: Option<String>,
    pub summary_commit: Option<String>,
    pub considered_message_count: i32,
    pub completed_at: Option<time::OffsetDateTime>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryCurateEnqueuedPayload {
    pub reflection_run_id: Uuid,
    pub lane: String,
    pub trigger: String,
    pub status: String,
    pub proposal_ids: Vec<Uuid>,
    pub conversation_key: Option<String>,
    pub conversation_date: Option<time::Date>,
    pub created_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryCurateStartedPayload {
    pub reflection_run_id: Uuid,
    pub lane: String,
    pub trigger: String,
    pub status: String,
    pub proposal_ids: Vec<Uuid>,
    pub conversation_key: Option<String>,
    pub conversation_date: Option<time::Date>,
    pub started_at: Option<time::OffsetDateTime>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryCurateCompletedPayload {
    pub reflection_run_id: Uuid,
    pub lane: String,
    pub trigger: String,
    pub status: String,
    pub proposal_ids: Vec<Uuid>,
    pub conversation_key: Option<String>,
    pub conversation_date: Option<time::Date>,
    pub completed_at: Option<time::OffsetDateTime>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryCurateFailedPayload {
    pub reflection_run_id: Uuid,
    pub lane: String,
    pub trigger: String,
    pub status: String,
    pub proposal_ids: Vec<Uuid>,
    pub conversation_key: Option<String>,
    pub conversation_date: Option<time::Date>,
    pub error: Option<String>,
    pub completed_at: Option<time::OffsetDateTime>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryReviewRequestedPayload {
    pub proposal_id: Uuid,
    pub source_role: String,
    pub title: String,
    pub suggested_action: String,
    pub status: String,
    pub source_paths: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ProjectionEvent {
    MemoryProposalCreated(MemoryProposalCreatedPayload),
    MemoryProposalResolved(MemoryProposalResolvedPayload),
    PairReflectionCompleted(PairReflectionCompletedPayload),
    MemoryCurateEnqueued(MemoryCurateEnqueuedPayload),
    MemoryCurateStarted(MemoryCurateStartedPayload),
    MemoryCurateCompleted(MemoryCurateCompletedPayload),
    MemoryCurateFailed(MemoryCurateFailedPayload),
    MemoryReviewRequested(MemoryReviewRequestedPayload),
}

impl ProjectionEvent {
    pub fn kind(&self) -> ProjectionEventKind {
        match self {
            Self::MemoryProposalCreated(_) => ProjectionEventKind::MemoryProposalCreated,
            Self::MemoryProposalResolved(_) => ProjectionEventKind::MemoryProposalResolved,
            Self::PairReflectionCompleted(_) => ProjectionEventKind::PairReflectionCompleted,
            Self::MemoryCurateEnqueued(_) => ProjectionEventKind::MemoryCurateEnqueued,
            Self::MemoryCurateStarted(_) => ProjectionEventKind::MemoryCurateStarted,
            Self::MemoryCurateCompleted(_) => ProjectionEventKind::MemoryCurateCompleted,
            Self::MemoryCurateFailed(_) => ProjectionEventKind::MemoryCurateFailed,
            Self::MemoryReviewRequested(_) => ProjectionEventKind::MemoryReviewRequested,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Projection {
    pub provenance: ProjectionProvenance,
    pub event: ProjectionEvent,
    pub workflow_text: String,
    pub visible_summary: Option<String>,
}

pub fn memory_proposal_created_projection(
    provenance: ProjectionProvenance,
    proposal_id: Uuid,
    source_role: String,
    suggested_action: String,
    title: String,
    status: String,
) -> Projection {
    Projection {
        provenance,
        event: ProjectionEvent::MemoryProposalCreated(MemoryProposalCreatedPayload {
            proposal_id,
            source_role: source_role.clone(),
            suggested_action,
            title: title.clone(),
            status,
        }),
        workflow_text: format!("Memory proposal created: {title}"),
        visible_summary: Some(format!(
            "Review requested for memory proposal '{}' from {}.",
            title, source_role
        )),
    }
}

pub fn memory_proposal_resolved_projection(
    provenance: ProjectionProvenance,
    proposal_id: Uuid,
    source_role: String,
    suggested_action: String,
    title: String,
    status: String,
    reviewer_role: Option<String>,
    result_path: Option<String>,
    result_commit: Option<String>,
) -> Projection {
    let visible_summary = match status.as_str() {
        "approved" => Some(match result_path.as_deref() {
            Some(path) => format!("Memory proposal '{}' was approved and applied at {path}.", title),
            None => format!("Memory proposal '{}' was approved.", title),
        }),
        "rejected" => Some(format!("Memory proposal '{}' was rejected.", title)),
        _ => None,
    };
    Projection {
        provenance,
        event: ProjectionEvent::MemoryProposalResolved(MemoryProposalResolvedPayload {
            proposal_id,
            source_role,
            suggested_action,
            title: title.clone(),
            status: status.clone(),
            reviewer_role,
            result_path,
            result_commit,
        }),
        workflow_text: format!("Memory proposal resolved: {} ({})", title, status),
        visible_summary,
    }
}

pub fn memory_review_requested_projection(
    provenance: ProjectionProvenance,
    proposal_id: Uuid,
    source_role: String,
    suggested_action: String,
    title: String,
    status: String,
    source_paths: Vec<String>,
) -> Projection {
    Projection {
        provenance,
        event: ProjectionEvent::MemoryReviewRequested(MemoryReviewRequestedPayload {
            proposal_id,
            source_role: source_role.clone(),
            title: title.clone(),
            suggested_action,
            status,
            source_paths,
        }),
        workflow_text: format!("Memory review requested: {title}"),
        visible_summary: Some(format!(
            "Review requested for memory proposal '{}' from {}.",
            title, source_role
        )),
    }
}

pub fn memory_curate_enqueued_projection(
    provenance: ProjectionProvenance,
    reflection_run_id: Uuid,
    lane: String,
    trigger: String,
    status: String,
    proposal_ids: Vec<Uuid>,
    conversation_key: Option<String>,
    conversation_date: Option<time::Date>,
    created_at: time::OffsetDateTime,
) -> Projection {
    let proposal_count = proposal_ids.len();
    Projection {
        provenance,
        event: ProjectionEvent::MemoryCurateEnqueued(MemoryCurateEnqueuedPayload {
            reflection_run_id,
            lane,
            trigger,
            status,
            proposal_ids,
            conversation_key,
            conversation_date,
            created_at,
        }),
        workflow_text: format!("Memory curate enqueued with {proposal_count} proposal(s)"),
        visible_summary: Some(format!("Memory curate was queued for {proposal_count} proposal(s).")),
    }
}

pub fn memory_curate_started_projection(
    provenance: ProjectionProvenance,
    reflection_run_id: Uuid,
    lane: String,
    trigger: String,
    status: String,
    proposal_ids: Vec<Uuid>,
    conversation_key: Option<String>,
    conversation_date: Option<time::Date>,
    started_at: Option<time::OffsetDateTime>,
) -> Projection {
    let proposal_count = proposal_ids.len();
    Projection {
        provenance,
        event: ProjectionEvent::MemoryCurateStarted(MemoryCurateStartedPayload {
            reflection_run_id,
            lane,
            trigger,
            status,
            proposal_ids,
            conversation_key,
            conversation_date,
            started_at,
        }),
        workflow_text: format!("Memory curate started with {proposal_count} proposal(s)"),
        visible_summary: Some(format!("Memory curate started for {proposal_count} proposal(s).")),
    }
}

pub fn memory_curate_completed_projection(
    provenance: ProjectionProvenance,
    reflection_run_id: Uuid,
    lane: String,
    trigger: String,
    status: String,
    proposal_ids: Vec<Uuid>,
    conversation_key: Option<String>,
    conversation_date: Option<time::Date>,
    completed_at: Option<time::OffsetDateTime>,
) -> Projection {
    let proposal_count = proposal_ids.len();
    Projection {
        provenance,
        event: ProjectionEvent::MemoryCurateCompleted(MemoryCurateCompletedPayload {
            reflection_run_id,
            lane,
            trigger,
            status,
            proposal_ids,
            conversation_key,
            conversation_date,
            completed_at,
        }),
        workflow_text: format!("Memory curate completed with {proposal_count} proposal(s)"),
        visible_summary: Some(format!("Memory curate completed for {proposal_count} proposal(s).")),
    }
}

pub fn memory_curate_failed_projection(
    provenance: ProjectionProvenance,
    reflection_run_id: Uuid,
    lane: String,
    trigger: String,
    status: String,
    proposal_ids: Vec<Uuid>,
    conversation_key: Option<String>,
    conversation_date: Option<time::Date>,
    error: Option<String>,
    completed_at: Option<time::OffsetDateTime>,
) -> Projection {
    let proposal_count = proposal_ids.len();
    let visible_summary = Some(match error.as_deref() {
        Some(message) if !message.trim().is_empty() => format!(
            "Memory curate failed for {proposal_count} proposal(s): {message}"
        ),
        _ => format!("Memory curate failed for {proposal_count} proposal(s)."),
    });
    Projection {
        provenance,
        event: ProjectionEvent::MemoryCurateFailed(MemoryCurateFailedPayload {
            reflection_run_id,
            lane,
            trigger,
            status,
            proposal_ids,
            conversation_key,
            conversation_date,
            error,
            completed_at,
        }),
        workflow_text: format!("Memory curate failed with {proposal_count} proposal(s)"),
        visible_summary,
    }
}

impl Projection {
    pub fn workflow_content_json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(&self.event).expect("projection event should serialize");
        let object = value
            .as_object_mut()
            .expect("projection event should serialize to object");
        object.insert(
            "source".to_string(),
            serde_json::json!(self.provenance.source.as_str()),
        );
        object.insert(
            "scope_id".to_string(),
            serde_json::json!(self.provenance.scope_id),
        );
        value
    }

    pub fn workflow_record(&self) -> CanonicalConversationRecord {
        CanonicalConversationRecord::workflow_event(
            self.workflow_text.clone(),
            self.workflow_content_json(),
            None,
        )
    }

    pub fn visible_summary_record(&self) -> Option<CanonicalConversationRecord> {
        self.visible_summary.as_ref().map(|text| {
            CanonicalConversationRecord::visible_assistant_message(
                text.clone(),
                serde_json::json!({}),
                None,
            )
        })
    }
}

pub async fn persist_projection(
    context: &ConversationPersistenceContext,
    projection: &Projection,
) -> Result<(), CustomError> {
    persist_canonical_conversation_record(context, &projection.workflow_record()).await?;
    if let Some(record) = projection.visible_summary_record() {
        persist_canonical_conversation_record(context, &record).await?;
    }
    Ok(())
}

pub fn spawn_persist_projection(
    context: ConversationPersistenceContext,
    projection: Projection,
) {
    if context.skip_persistence {
        return;
    }
    let span_request_id = context.request_id.clone();
    let span_scope = context.persistence_scope_id.clone();
    tokio::spawn(
        async move {
            if let Err(err) = persist_projection(&context, &projection).await {
                tracing::warn!(
                    request_id = ?context.request_id,
                    persistence_scope_id = %context.persistence_scope_id,
                    error = %err,
                    "projection persistence failed"
                );
            }
        }
        .instrument(tracing::info_span!(
            "persist_projection",
            request_id = ?span_request_id,
            persistence_scope_id = %span_scope,
        )),
    );
}

pub fn project_to_conversation(
    pool: &PgPool,
    bear_id: Uuid,
    user_id: Option<i32>,
    conversation_id: Option<&str>,
    projection: Projection,
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
        projection.provenance.scope_id.clone(),
        false,
    );
    spawn_persist_projection(context, projection);
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

