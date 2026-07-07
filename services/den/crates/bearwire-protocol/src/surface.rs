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
    pub const fn kind(&self) -> &'static str {
        match self {
            SurfaceHistoryEvent::Message { .. } => "message",
            SurfaceHistoryEvent::ToolCall { .. } => "tool_call",
            SurfaceHistoryEvent::ToolResult { .. } => "tool_result",
            SurfaceHistoryEvent::ReasoningDelta { .. } => "reasoning_delta",
            SurfaceHistoryEvent::SessionInfoUpdate { .. } => "session_info_update",
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_surface_records_validate() {
        let records = [
            SurfaceHistoryEvent::Message {
                id: Some("m1".to_string()),
                role: "user".to_string(),
                text: "hello".to_string(),
                created_at: Some("2026-07-07T00:00:00Z".to_string()),
            },
            SurfaceHistoryEvent::ToolCall {
                id: Some("call-1".to_string()),
                role: Some("assistant".to_string()),
                tool_call_id: "call-1".to_string(),
                tool_name: "fs_read_text_file".to_string(),
                status: "pending".to_string(),
                arguments: json!({ "path": "README.md" }),
                created_at: None,
            },
            SurfaceHistoryEvent::ToolResult {
                id: Some("call-1".to_string()),
                role: Some("tool".to_string()),
                tool_call_id: "call-1".to_string(),
                tool_name: "fs_read_text_file".to_string(),
                status: "ok".to_string(),
                text: Some("Read file.".to_string()),
                raw_output: json!({ "content": "hello" }),
                created_at: None,
            },
            SurfaceHistoryEvent::ReasoningDelta {
                id: Some("r1".to_string()),
                role: Some("assistant".to_string()),
                text: "thinking".to_string(),
                source: Some("provider_reasoning".to_string()),
                replay_policy: Some("thought".to_string()),
                created_at: None,
            },
            SurfaceHistoryEvent::SessionInfoUpdate {
                id: Some("s1".to_string()),
                role: Some("system".to_string()),
                session_id: Some("session-1".to_string()),
                title: Some("Title".to_string()),
                title_updated_at: None,
                current_mode: Some("write".to_string()),
                created_at: None,
            },
        ];

        for record in records {
            record
                .validate_replay_record()
                .unwrap_or_else(|err| panic!("{} should validate: {err}", record.kind()));
        }
    }

    #[test]
    fn invalid_surface_records_are_rejected() {
        let cases = [
            (
                SurfaceHistoryEvent::Message {
                    id: None,
                    role: "".to_string(),
                    text: "hello".to_string(),
                    created_at: None,
                },
                "message missing required role",
            ),
            (
                SurfaceHistoryEvent::ToolCall {
                    id: None,
                    role: None,
                    tool_call_id: "".to_string(),
                    tool_name: "tool".to_string(),
                    status: "pending".to_string(),
                    arguments: Value::Null,
                    created_at: None,
                },
                "tool record missing required tool_call_id",
            ),
            (
                SurfaceHistoryEvent::ReasoningDelta {
                    id: None,
                    role: None,
                    text: "".to_string(),
                    source: None,
                    replay_policy: Some("thought".to_string()),
                    created_at: None,
                },
                "reasoning record missing required text",
            ),
            (
                SurfaceHistoryEvent::SessionInfoUpdate {
                    id: None,
                    role: None,
                    session_id: None,
                    title: None,
                    title_updated_at: None,
                    current_mode: None,
                    created_at: None,
                },
                "session info update missing title or title_updated_at",
            ),
        ];

        for (record, expected) in cases {
            assert_eq!(record.validate_replay_record(), Err(expected));
        }
    }

    #[test]
    fn message_created_at_must_decode_as_string() {
        let err = serde_json::from_value::<SurfaceHistoryEvent>(json!({
            "kind": "message",
            "role": "user",
            "text": "hello",
            "created_at": [2026, 7, 7]
        }))
        .expect_err("array timestamp should not decode as string created_at");
        assert!(err.to_string().contains("expected a string"), "{err}");
    }
}
