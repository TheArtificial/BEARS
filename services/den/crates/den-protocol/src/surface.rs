use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceHistoryEvent {
    Message {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        role: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<String>,
    },
    ToolCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        tool_call_id: String,
        tool_name: String,
        status: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        arguments: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<String>,
    },
    ToolResult {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        tool_call_id: String,
        tool_name: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        raw_output: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<String>,
    },
    #[serde(alias = "reasoning")]
    ReasoningDelta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replay_policy: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<String>,
    },
    SessionInfoUpdate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title_updated_at: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<String>,
    },
}

impl SurfaceHistoryEvent {
    pub fn validate_replay_record(&self) -> Result<(), &'static str> {
        match self {
            SurfaceHistoryEvent::Message { role, text, .. } => {
                if role.trim().is_empty() {
                    return Err("message missing required role");
                }
                if text.trim().is_empty() {
                    return Err("message missing required text");
                }
            }
            SurfaceHistoryEvent::ToolCall {
                tool_call_id,
                tool_name,
                status,
                ..
            }
            | SurfaceHistoryEvent::ToolResult {
                tool_call_id,
                tool_name,
                status,
                ..
            } => {
                if tool_call_id.trim().is_empty() {
                    return Err("tool record missing required tool_call_id");
                }
                if tool_name.trim().is_empty() {
                    return Err("tool record missing required tool_name");
                }
                if status.trim().is_empty() {
                    return Err("tool record missing required status");
                }
            }
            SurfaceHistoryEvent::ReasoningDelta { text, .. } => {
                if text.trim().is_empty() {
                    return Err("reasoning record missing required text");
                }
            }
            SurfaceHistoryEvent::SessionInfoUpdate {
                title,
                title_updated_at,
                ..
            } => {
                if title.as_deref().unwrap_or_default().trim().is_empty()
                    && title_updated_at
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    return Err("session info update missing title or title_updated_at");
                }
            }
        }
        Ok(())
    }
}
