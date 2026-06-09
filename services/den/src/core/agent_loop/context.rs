use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    core::llm::{ChatMessage, ChatToolCall, ChatToolCallFunction},
    errors::CustomError,
};

#[derive(Debug, Clone)]
struct TranscriptRow {
    message_type: String,
    content_text: String,
    content_json: Value,
}

fn reconstruct_transcript_messages(rows: Vec<TranscriptRow>) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    for row in rows {
        match row.message_type.as_str() {
            "user" if !row.content_text.trim().is_empty() => {
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: Some(row.content_text),
                    tool_call_id: None,
                    name: None,
                    tool_calls: None,
                });
            }
            "assistant" if !row.content_text.trim().is_empty() => {
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: Some(row.content_text),
                    tool_call_id: None,
                    name: None,
                    tool_calls: None,
                });
            }
            "tool_call" => {
                let Some(event) = row.content_json.get("event").and_then(Value::as_str) else {
                    continue;
                };
                if event != "tool_request" {
                    continue;
                }
                let tool_call_id = row
                    .content_json
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let tool_name = row
                    .content_json
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let arguments = row
                    .content_json
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Default::default()));
                let call = ChatToolCall {
                    id: tool_call_id,
                    call_type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: tool_name,
                        arguments: arguments.to_string(),
                    },
                };
                if let Some(last) = messages.last_mut() {
                    if last.role == "assistant" {
                        last.tool_calls
                            .get_or_insert_with(Vec::new)
                            .push(call);
                        continue;
                    }
                }
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_call_id: None,
                    name: None,
                    tool_calls: Some(vec![call]),
                });
            }
            "tool_result" => {
                let Some(event) = row.content_json.get("event").and_then(Value::as_str) else {
                    continue;
                };
                if event != "tool_result" {
                    continue;
                }
                let tool_call_id = row
                    .content_json
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let content = row
                    .content_json
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        if row.content_text.trim().is_empty() {
                            None
                        } else {
                            Some(row.content_text.clone())
                        }
                    });
                if content.is_none() && tool_call_id.is_none() {
                    continue;
                }
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content,
                    tool_call_id,
                    name: row
                        .content_json
                        .get("tool_name")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    tool_calls: None,
                });
            }
            _ => {}
        }
    }
    messages
}

pub async fn load_transcript_messages(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
) -> Result<Vec<ChatMessage>, CustomError> {
    let history_rows = sqlx::query_as::<_, (String, String, Value)>(
        r#"
        SELECT message_type, content_text, content_json
        FROM conversation_messages
        WHERE conversation_id = (
            SELECT id FROM conversations
            WHERE external_conversation_id = $1 AND bear_id = $2
            LIMIT 1
        )
        AND (
            visibility != 'diagnostic_only'
            OR message_type IN ('tool_call', 'tool_result')
        )
        ORDER BY sequence_no ASC
        LIMIT 80
        "#,
    )
    .bind(conversation_id)
    .bind(bear_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let rows = history_rows
        .into_iter()
        .map(|(message_type, content_text, content_json)| TranscriptRow {
            message_type,
            content_text,
            content_json,
        })
        .collect();
    Ok(reconstruct_transcript_messages(rows))
}

pub async fn assemble_agent_messages(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    system_context: Option<&str>,
    human_message: Option<&str>,
    tool_messages: &[ChatMessage],
) -> Result<Vec<ChatMessage>, CustomError> {
    let mut messages = Vec::new();
    if let Some(system) = system_context.filter(|s| !s.is_empty()) {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(system.to_string()),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        });
    }
    messages.extend(load_transcript_messages(pool, bear_id, conversation_id).await?);
    if let Some(human) = human_message.filter(|s| !s.is_empty()) {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: Some(human.to_string()),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        });
    }
    messages.extend(tool_messages.iter().cloned());
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reconstruct_transcript_messages_rebuilds_assistant_tool_and_result_sequence() {
        let rows = vec![
            TranscriptRow {
                message_type: "user".to_string(),
                content_text: "read file".to_string(),
                content_json: json!({}),
            },
            TranscriptRow {
                message_type: "assistant".to_string(),
                content_text: "I'll read it".to_string(),
                content_json: json!({}),
            },
            TranscriptRow {
                message_type: "tool_call".to_string(),
                content_text: "Tool request: fs.read".to_string(),
                content_json: json!({
                    "event": "tool_request",
                    "tool_call_id": "call_1",
                    "tool_name": "fs.read",
                    "args": {"path": "/tmp/a"},
                }),
            },
            TranscriptRow {
                message_type: "tool_result".to_string(),
                content_text: "Tool result: fs.read".to_string(),
                content_json: json!({
                    "event": "tool_result",
                    "tool_call_id": "call_1",
                    "tool_name": "fs.read",
                    "content": "hello",
                }),
            },
            TranscriptRow {
                message_type: "assistant".to_string(),
                content_text: "done".to_string(),
                content_json: json!({}),
            },
        ];
        let messages = reconstruct_transcript_messages(rows);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(
            messages[1].tool_calls.as_ref().map(|calls| calls.len()),
            Some(1)
        );
        assert_eq!(messages[2].role, "tool");
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(messages[3].role, "assistant");
    }
}
