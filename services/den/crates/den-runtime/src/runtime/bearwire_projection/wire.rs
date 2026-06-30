use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use den_protocol::{
    RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent, ToolCallFinishStatus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BearWireEventScope {
    Persistent,
    Ephemeral,
}

impl Default for BearWireEventScope {
    fn default() -> Self {
        Self::Ephemeral
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BearWireEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    pub scope: BearWireEventScope,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bear_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_refs: Vec<ResourceRef>,
    pub data: Value,
}

impl BearWireEvent {
    pub fn ephemeral(event_type: impl Into<String>, data: Value) -> Self {
        Self {
            event_id: None,
            sequence: None,
            scope: BearWireEventScope::Ephemeral,
            source: "den.runtime".to_string(),
            event_type: event_type.into(),
            subject: None,
            time: None,
            bear_id: None,
            role: None,
            role_agent_id: None,
            human_id: None,
            session_id: None,
            run_id: None,
            resource_refs: Vec::new(),
            data,
        }
    }

    fn with_run_id(mut self, run_id: Option<String>) -> Self {
        self.run_id = run_id.clone();
        if let Some(run_id) = run_id {
            self.subject = Some(format!("resource/run/{run_id}"));
            self.resource_refs.push(ResourceRef::new("run", run_id));
        }
        self
    }

    fn with_tool_call(mut self, tool_call_id: String) -> Self {
        self.subject = Some(format!("resource/tool_call/{tool_call_id}"));
        self.resource_refs
            .push(ResourceRef::new("tool_call", tool_call_id));
        self
    }

    fn with_permission_request(mut self, permission_request_id: Option<String>) -> Self {
        if let Some(permission_request_id) = permission_request_id {
            self.resource_refs.push(ResourceRef::new(
                "permission_request",
                permission_request_id,
            ));
        }
        self
    }

    fn with_session(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id.clone());
        self.subject = Some(format!("resource/session/{session_id}"));
        self.resource_refs
            .push(ResourceRef::new("session", session_id));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRef {
    pub kind: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl ResourceRef {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            uri: None,
            display_name: None,
            version: None,
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcNotification<T> {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub params: T,
}

pub fn bearwire_event_to_json_rpc_notification(
    event: BearWireEvent,
) -> JsonRpcNotification<BearWireEvent> {
    JsonRpcNotification {
        jsonrpc: "2.0",
        method: "event",
        params: event,
    }
}

pub fn runtime_stream_event_to_bearwire_events(event: RuntimeStreamEvent) -> Vec<BearWireEvent> {
    match event {
        RuntimeStreamEvent::Semantic(event) => runtime_semantic_event_to_bearwire_events(event),
        RuntimeStreamEvent::UntranslatedProviderEvent { .. } => Vec::new(),
    }
}

pub fn runtime_stream_event_to_bearwire_notifications(
    event: RuntimeStreamEvent,
) -> Vec<JsonRpcNotification<BearWireEvent>> {
    runtime_stream_event_to_bearwire_events(event)
        .into_iter()
        .map(bearwire_event_to_json_rpc_notification)
        .collect()
}

pub fn runtime_semantic_event_to_bearwire_events(
    event: RuntimeSemanticEvent,
) -> Vec<BearWireEvent> {
    match event {
        RuntimeSemanticEvent::AssistantTextDelta { text } => vec![BearWireEvent::ephemeral(
            "message.delta",
            json!({
                "delta": text,
            }),
        )],
        RuntimeSemanticEvent::StatusText { text } => vec![BearWireEvent::ephemeral(
            "run.progress",
            json!({
                "kind": "status_text",
                "text": text,
            }),
        )],
        RuntimeSemanticEvent::RunProgress {
            kind,
            text,
            phase,
            detail,
        } => vec![BearWireEvent::ephemeral(
            "run.progress",
            json!({
                "kind": kind,
                "text": text,
                "phase": phase,
                "detail": detail,
            }),
        )],
        RuntimeSemanticEvent::RunPaused {
            reason,
            resume_token,
            expires_at,
        } => vec![BearWireEvent::ephemeral(
            "run.paused",
            json!({
                "reason": reason,
                "resume_token": resume_token,
                "expires_at": expires_at,
            }),
        )],
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            title,
            kind,
            arguments,
            approval_request_id,
            approval_required,
            approval_reason,
            run_id,
        } => {
            let has_permission_id = approval_request_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|id| !id.is_empty());
            let event_type = if approval_required && has_permission_id {
                "tool_call.blocked"
            } else {
                "tool_call.requested"
            };
            vec![BearWireEvent::ephemeral(
                event_type,
                json!({
                    "tool_call_id": tool_call_id,
                    "tool_name": tool_name,
                    "title": title,
                    "kind": kind.unwrap_or_else(|| "function".to_string()),
                    "arguments": arguments,
                    "approval_required": approval_required && has_permission_id,
                    "approval_request_id": approval_request_id,
                    "reason": approval_reason,
                }),
            )
            .with_run_id(run_id)
            .with_tool_call(tool_call_id)
            .with_permission_request(approval_request_id)]
        }
        RuntimeSemanticEvent::ToolCallFinished {
            tool_call_id,
            tool_name,
            status,
            summary,
            error_message,
        } => {
            let event_type = match status {
                ToolCallFinishStatus::Ok => "tool_call.completed",
                ToolCallFinishStatus::Error => "tool_call.failed",
                ToolCallFinishStatus::Incomplete => "tool_call.warning",
                ToolCallFinishStatus::Cancelled => "tool_call.cancelled",
            };
            vec![BearWireEvent::ephemeral(
                event_type,
                json!({
                    "tool_call_id": tool_call_id,
                    "tool_name": tool_name,
                    "status": status.as_str(),
                    "summary": summary,
                    "error_message": error_message,
                }),
            )
            .with_tool_call(tool_call_id)]
        }
        RuntimeSemanticEvent::Error {
            message,
            detail,
            error_type,
            request_id,
            context,
        } => vec![BearWireEvent::ephemeral(
            "run.failed",
            json!({
                "message": message,
                "detail": detail,
                "error_type": error_type,
                "request_id": request_id,
                "context": context,
            }),
        )],
        RuntimeSemanticEvent::ConversationResolved { conversation } => {
            vec![BearWireEvent::ephemeral(
                "session.bound",
                json!({
                    "binding": {
                        "conversation_id": conversation.id,
                    }
                }),
            )
            .with_session(conversation.id)]
        }
        RuntimeSemanticEvent::TurnCompleted { turn } => vec![BearWireEvent::ephemeral(
            "run.completed",
            json!({
                "outcome": "ok",
                "turn": turn,
            }),
        )],
        RuntimeSemanticEvent::TurnFailed {
            turn,
            category,
            message,
        } => vec![BearWireEvent::ephemeral(
            "run.failed",
            json!({
                "reason": runtime_error_category_wire(&category),
                "message": message,
                "turn": turn,
            }),
        )],
        RuntimeSemanticEvent::TurnCancelled { turn } => vec![BearWireEvent::ephemeral(
            "run.cancelled",
            json!({
                "reason": "runtime_turn_cancelled",
                "turn": turn,
            }),
        )],
    }
}

fn runtime_error_category_wire(category: &RuntimeErrorCategory) -> &'static str {
    match category {
        RuntimeErrorCategory::Unavailable => "runtime_unavailable",
        RuntimeErrorCategory::Misconfigured => "runtime_misconfigured",
        RuntimeErrorCategory::InvalidIdentity => "runtime_invalid_identity",
        RuntimeErrorCategory::PermissionDenied => "runtime_permission_denied",
        RuntimeErrorCategory::ConflictPendingApproval => "runtime_conflict_pending_approval",
        RuntimeErrorCategory::Cancelled => "runtime_cancelled",
        RuntimeErrorCategory::Timeout => "runtime_timeout",
        RuntimeErrorCategory::BackendProtocol => "runtime_backend_protocol",
        RuntimeErrorCategory::Internal => "runtime_internal",
    }
}
