use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use den_core::{
    client_tools::client_tool_display_for_provider,
    tools::descriptor::den_tool_display_json_for_provider,
};
use den_protocol::{
    RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BearWireEventScope {
    Persistent,
    #[default]
    Ephemeral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    pub fn ephemeral_typed<T: Serialize>(event_type: impl Into<String>, data: T) -> Self {
        Self::ephemeral(
            event_type,
            serde_json::to_value(data).expect("BearWire typed event data serializes"),
        )
    }

    pub fn tool_call_requested(data: ToolCallRequestedWire) -> Self {
        Self::ephemeral_typed("tool_call.requested", data)
    }

    pub fn tool_call_waiting(data: ToolCallWaitingWire) -> Self {
        Self::ephemeral_typed("client.waiting", data)
    }

    pub fn tool_call_finished(data: ToolCallFinishWire) -> Self {
        let event_type = match data.status {
            ToolCallFinishStatusWire::Ok => "tool_call.completed",
            ToolCallFinishStatusWire::Incomplete => "tool_call.warning",
            ToolCallFinishStatusWire::Cancelled => "tool_call.cancelled",
            ToolCallFinishStatusWire::Error => "tool_call.failed",
        };
        Self::ephemeral_typed(event_type, data)
    }

    fn with_run_id(mut self, run_id: Option<String>) -> Self {
        self.run_id.clone_from(&run_id);
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcNotification<T> {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub params: T,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallWire {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub kind: String,
    pub arguments: Value,
    pub display: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRefWire {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallFinishStatusWire {
    Ok,
    Error,
    Incomplete,
    Cancelled,
}

impl ToolCallFinishStatusWire {
    pub fn from_wire_str(status: &str) -> Self {
        match status {
            "ok" => Self::Ok,
            "incomplete" => Self::Incomplete,
            "cancelled" => Self::Cancelled,
            _ => Self::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallFinishWire {
    pub tool_call_id: String,
    pub tool_name: Option<String>,
    pub tool_call: ToolCallRefWire,
    pub status: ToolCallFinishStatusWire,
    pub summary: Option<String>,
    pub error_message: Option<String>,
    pub content: Option<String>,
    pub structured_content: Option<Value>,
    pub error: Option<Value>,
    pub compacted: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPermissionWire {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRequestedWire {
    pub tool_call_id: String,
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub display: Value,
    pub kind: String,
    pub arguments: Value,
    pub tool_call: ToolCallWire,
    pub approval_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallWaitingWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_responder_action: Option<String>,
    pub expected_client_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obligation_id: Option<String>,
    pub tool_call_id: String,
    pub tool_name: String,
    pub tool_call: ToolCallWire,
    pub permission: ToolPermissionWire,
    pub approval_required: bool,
    pub approval_request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_step_id: Option<String>,
}

pub fn tool_call_wire(
    tool_call_id: &str,
    tool_name: &str,
    title: Option<&str>,
    kind: &str,
    arguments: &Value,
) -> ToolCallWire {
    let display = den_tool_display_json_for_provider(tool_name, arguments)
        .unwrap_or_else(|| client_tool_display_for_provider(tool_name, arguments));
    ToolCallWire {
        id: tool_call_id.to_string(),
        name: tool_name.to_string(),
        title: title.map(str::to_string),
        kind: kind.to_string(),
        arguments: arguments.clone(),
        display,
    }
}

pub fn tool_call_finish_wire(
    tool_call_id: &str,
    tool_name: Option<&str>,
    status: &str,
    summary: Option<&str>,
    error_message: Option<&str>,
    content: Option<&str>,
    structured_content: Option<Value>,
    error: Option<Value>,
    compacted: Option<Value>,
) -> ToolCallFinishWire {
    // ponytail: content preview is deliberately simple; upgrade by sharing the
    // client-tool result compactor's summary extraction if cards need richer text.
    let summary = summary
        .map(str::to_string)
        .or_else(|| error_message.map(str::to_string))
        .or_else(|| content.map(|text| text.chars().take(160).collect::<String>()));
    let tool_name = tool_name.map(str::to_string);
    ToolCallFinishWire {
        tool_call_id: tool_call_id.to_string(),
        tool_name: tool_name.clone(),
        tool_call: ToolCallRefWire {
            id: tool_call_id.to_string(),
            name: tool_name,
        },
        status: ToolCallFinishStatusWire::from_wire_str(status),
        summary,
        error_message: error_message.map(str::to_string),
        content: content.map(str::to_string),
        structured_content,
        error,
        compacted,
    }
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
        RuntimeSemanticEvent::ReasoningTextDelta { text } => vec![BearWireEvent::ephemeral(
            "message.reasoning.delta",
            json!({
                "delta": text,
                "source": "provider_reasoning",
                "replay_policy": "none",
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
        } => {
            if kind == "session_info_update" {
                let title = detail
                    .as_ref()
                    .and_then(|value| value.get("title"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let updated_at = detail
                    .as_ref()
                    .and_then(|value| value.get("updated_at"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                return vec![BearWireEvent::ephemeral(
                    "session_info_update",
                    json!({
                        "title": title,
                        "updated_at": updated_at,
                    }),
                )];
            }
            vec![BearWireEvent::ephemeral(
                "run.progress",
                json!({
                    "kind": kind,
                    "text": text,
                    "phase": phase,
                    "detail": detail,
                }),
            )]
        }
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
            let effective_kind = kind.unwrap_or_else(|| "function".to_string());
            let tool_call = tool_call_wire(
                &tool_call_id,
                &tool_name,
                title.as_deref(),
                &effective_kind,
                &arguments,
            );
            let event = if approval_required && has_permission_id {
                let permission_id = approval_request_id.clone().unwrap_or_default();
                BearWireEvent::tool_call_waiting(ToolCallWaitingWire {
                    expected_responder_action: None,
                    expected_client_method: "client.permission.result".to_string(),
                    obligation_id: None,
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    tool_call,
                    permission: ToolPermissionWire {
                        id: permission_id.clone(),
                        reason: approval_reason.clone(),
                        title: None,
                        target: None,
                    },
                    approval_required: true,
                    approval_request_id: permission_id,
                    permission_id: None,
                    turn_step_id: None,
                })
            } else {
                BearWireEvent::tool_call_requested(ToolCallRequestedWire {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    title,
                    display: tool_call.display.clone(),
                    kind: effective_kind,
                    arguments,
                    tool_call,
                    approval_required: false,
                    approval_request_id: approval_request_id.clone(),
                    reason: approval_reason,
                })
            };
            vec![event
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
            vec![BearWireEvent::tool_call_finished(tool_call_finish_wire(
                &tool_call_id,
                Some(&tool_name),
                status.as_str(),
                summary.as_deref(),
                error_message.as_deref(),
                None,
                None,
                None,
                None,
            ))
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
