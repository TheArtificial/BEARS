use den_core::DenError;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    {
        agent_loop::tool_outcome::{
            is_legacy_synthetic_interrupted_tool_result,
            tool_message_counts_toward_llm_resolution, INCOMPLETE_TOOL_RESULT_MARK,
        },
        conversation_events::{
            canonical_persistence_context, spawn_persist_tool_result, ConversationEventProvenance,
        },
        llm::{ChatMessage, ChatToolCall, ChatToolCallFunction},
        runtime::compaction::{
            semantic_groups_from_conversation_messages, TranscriptGroupingRow,
        },
        runtime_conversations::RuntimeSemanticGroup,
    },
};

#[derive(Debug, Clone)]
struct TranscriptRow {
    message_type: String,
    content_text: String,
    content_json: Value,
}

type TranscriptHistoryRow = (
    String,
    i64,
    String,
    String,
    Value,
    Option<String>,
);

const TRANSCRIPT_HISTORY_QUERY: &str = r"
        SELECT id::text, sequence_no, message_type, content_text, content_json, tool_call_id
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
        ";

fn transcript_grouping_rows_from_history(history_rows: Vec<TranscriptHistoryRow>) -> Vec<TranscriptGroupingRow> {
    history_rows
        .into_iter()
        .map(
            |(message_id, sequence_no, message_type, content_text, content_json, tool_call_id)| {
                TranscriptGroupingRow {
                    message_id: Some(message_id),
                    sequence_no: Some(sequence_no),
                    message_type,
                    content_text,
                    content_json,
                    tool_call_id,
                }
            },
        )
        .collect()
}

fn transcript_rows_from_grouping_rows(rows: &[TranscriptGroupingRow]) -> Vec<TranscriptRow> {
    rows.iter()
        .map(|row| TranscriptRow {
            message_type: row.message_type.clone(),
            content_text: row.content_text.clone(),
            content_json: row.content_json.clone(),
        })
        .collect()
}

fn assistant_has_tool_call(messages: &[ChatMessage], tool_call_id: &str) -> bool {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .and_then(|message| message.tool_calls.as_ref())
        .is_some_and(|calls| calls.iter().any(|call| call.id == tool_call_id))
}

fn reconstruct_transcript_messages(rows: Vec<TranscriptRow>) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    let mut pending_tool_results: std::collections::HashMap<String, ChatMessage> =
        std::collections::HashMap::new();
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
                    id: tool_call_id.clone(),
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
                        if let Some(pending) = pending_tool_results.remove(&tool_call_id) {
                            messages.push(pending);
                        }
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
                if let Some(pending) = pending_tool_results.remove(&tool_call_id) {
                    messages.push(pending);
                }
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
                let persisted_status = row
                    .content_json
                    .get("status")
                    .and_then(Value::as_str);
                let content = if persisted_status == Some("incomplete") {
                    Some(INCOMPLETE_TOOL_RESULT_MARK.to_string())
                } else {
                    row.content_json
                        .get("content")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            if row.content_text.trim().is_empty() {
                                None
                            } else {
                                Some(row.content_text.clone())
                            }
                        })
                };
                if content.is_none() && tool_call_id.is_none() {
                    continue;
                }
                let tool_message = ChatMessage {
                    role: "tool".to_string(),
                    content,
                    tool_call_id: tool_call_id.clone(),
                    name: row
                        .content_json
                        .get("tool_name")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    tool_calls: None,
                };
                if let Some(tool_call_id) = tool_call_id {
                    if assistant_has_tool_call(&messages, &tool_call_id) {
                        messages.push(tool_message);
                    } else {
                        pending_tool_results.insert(tool_call_id, tool_message);
                    }
                } else {
                    messages.push(tool_message);
                }
            }
            _ => {}
        }
    }
    messages
}

/// Drops assistant tool-call turns that never completed (interrupted turns, legacy synthetic
/// placeholders). Keeps only fully-resolved tool-call chains so future turns are not poisoned.
pub fn repair_tool_call_message_chain(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut repaired = Vec::with_capacity(messages.len());
    let mut index = 0;
    while index < messages.len() {
        let message = &messages[index];
        if message.role == "assistant"
            && message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| !calls.is_empty())
        {
            let assistant = message.clone();
            let required_ids: Vec<String> = assistant
                .tool_calls
                .as_ref()
                .expect("checked above")
                .iter()
                .map(|call| call.id.clone())
                .collect();
            index += 1;
            let mut tool_messages = Vec::new();
            let mut resolved_ids = std::collections::HashSet::new();
            let mut has_legacy_synthetic = false;
            while index < messages.len() && messages[index].role == "tool" {
                let tool_message = messages[index].clone();
                if is_legacy_synthetic_interrupted_tool_result(
                    tool_message.content.as_deref(),
                ) {
                    has_legacy_synthetic = true;
                }
                if let Some(tool_call_id) = tool_message.tool_call_id.as_deref() {
                    if tool_message_counts_toward_llm_resolution(tool_message.content.as_deref()) {
                        resolved_ids.insert(tool_call_id.to_string());
                    }
                }
                tool_messages.push(tool_message);
                index += 1;
            }
            let complete = !has_legacy_synthetic
                && required_ids
                    .iter()
                    .all(|tool_call_id| resolved_ids.contains(tool_call_id));
            if complete {
                repaired.push(assistant);
                repaired.extend(tool_messages);
            } else {
                tracing::debug!(
                    required_tool_results = required_ids.len(),
                    resolved_tool_results = resolved_ids.len(),
                    has_legacy_synthetic,
                    "dropping incomplete assistant tool-call turn from LLM context"
                );
            }
            continue;
        }

        if message.role == "tool" {
            tracing::debug!(
                tool_call_id = ?message.tool_call_id,
                "dropping orphan tool message from LLM context"
            );
            index += 1;
            continue;
        }

        repaired.push(message.clone());
        index += 1;
    }
    repaired
}

fn orphan_tool_call_ids_from_rows(rows: &[TranscriptRow]) -> Vec<(String, String)> {
    let mut requested = Vec::new();
    let mut resolved = std::collections::HashSet::new();
    for row in rows {
        let Some(event) = row.content_json.get("event").and_then(Value::as_str) else {
            continue;
        };
        match event {
            "tool_request" => {
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
                    .unwrap_or("tool")
                    .to_string();
                if !tool_call_id.is_empty() {
                    requested.push((tool_call_id, tool_name));
                }
            }
            "tool_result" => {
                if let Some(tool_call_id) = row
                    .content_json
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                {
                    resolved.insert(tool_call_id.to_string());
                }
            }
            _ => {}
        }
    }
    requested
        .into_iter()
        .filter(|(tool_call_id, _)| !resolved.contains(tool_call_id))
        .collect()
}

fn backfill_incomplete_tool_results(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    rows: &[TranscriptRow],
) {
    let orphans = orphan_tool_call_ids_from_rows(rows);
    if orphans.is_empty() {
        return;
    }
    let provenance = ConversationEventProvenance::acp_session(format!("den-web:{bear_id}:{conversation_id}"));
    let context = canonical_persistence_context(
        pool.clone(),
        bear_id,
        None,
        conversation_id.to_string(),
        Some(format!("den-web:{bear_id}:{conversation_id}")),
        None,
        format!("den-web:{bear_id}:{conversation_id}"),
        false,
    );
    for (tool_call_id, tool_name) in orphans {
        tracing::info!(
            %tool_call_id,
            %tool_name,
            %conversation_id,
            "backfilling incomplete tool_result for orphan tool_call"
        );
        spawn_persist_tool_result(
            context.clone(),
            Some(tool_name.clone()),
            tool_call_id,
            None,
            "incomplete".to_string(),
            None,
            Value::Null,
            serde_json::json!({
                "component": "den.agent_loop",
                "phase": "orphan_tool_call_backfill",
                "reason": "turn_interrupted",
                "tool_name": tool_name,
            }),
            None,
            &provenance,
        );
    }
}

pub async fn load_transcript_grouping_rows(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
) -> Result<Vec<TranscriptGroupingRow>, DenError> {
    let history_rows = sqlx::query_as::<_, TranscriptHistoryRow>(TRANSCRIPT_HISTORY_QUERY)
        .bind(conversation_id)
        .bind(bear_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    Ok(transcript_grouping_rows_from_history(history_rows))
}

/// Loads persisted transcript rows and derives semantic compaction groups.
///
/// Intended for Phase B prompt-assembler integration; not yet called from the live path.
#[expect(dead_code, reason = "Phase B assembler integration")]
pub async fn load_transcript_semantic_groups(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
) -> Result<Vec<RuntimeSemanticGroup>, DenError> {
    let rows = load_transcript_grouping_rows(pool, bear_id, conversation_id).await?;
    Ok(semantic_groups_from_conversation_messages(&rows))
}

pub async fn load_transcript_messages(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
) -> Result<Vec<ChatMessage>, DenError> {
    let grouping_rows = load_transcript_grouping_rows(pool, bear_id, conversation_id).await?;
    let rows = transcript_rows_from_grouping_rows(&grouping_rows);
    backfill_incomplete_tool_results(pool, bear_id, conversation_id, &rows);
    Ok(reconstruct_transcript_messages(rows))
}

/// Cap transcript tail sent to native browser chat turns (system prefix + recent turns).
pub fn prune_messages_for_native_chat(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    prune_messages_for_native_pair(messages)
}

/// Cap transcript tail sent to native pair LLM turns (system prefix + recent turns).
pub fn prune_messages_for_native_pair(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    const MAX_TAIL_MESSAGES: usize = 28;
    if messages.len() <= MAX_TAIL_MESSAGES {
        return messages;
    }
    let system = messages
        .first()
        .filter(|message| message.role == "system")
        .cloned();
    let mut tail_start = messages.len().saturating_sub(MAX_TAIL_MESSAGES - 1);
    while tail_start < messages.len() && messages[tail_start].role == "tool" {
        tail_start += 1;
    }
    let mut pruned = Vec::with_capacity(MAX_TAIL_MESSAGES);
    if let Some(system_message) = system {
        pruned.push(system_message);
    }
    pruned.extend(messages.into_iter().skip(tail_start));
    repair_tool_call_message_chain(pruned)
}

pub async fn assemble_agent_messages(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    system_context: Option<&str>,
    human_message: Option<&str>,
    tool_messages: &[ChatMessage],
) -> Result<Vec<ChatMessage>, DenError> {
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
    Ok(repair_tool_call_message_chain(messages))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loop::tool_outcome::LEGACY_SYNTHETIC_TOOL_RESULT_UNAVAILABLE;
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

    #[test]
    fn prune_messages_for_native_pair_keeps_system_and_recent_tail() {
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: Some("system".to_string()),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }];
        for index in 0..40 {
            messages.push(ChatMessage {
                role: if index % 2 == 0 {
                    "user".to_string()
                } else {
                    "assistant".to_string()
                },
                content: Some(format!("message-{index}")),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            });
        }
        let pruned = prune_messages_for_native_pair(messages);
        assert_eq!(pruned.first().map(|m| m.role.as_str()), Some("system"));
        assert_eq!(pruned.len(), 28);
        assert_eq!(
            pruned.last().and_then(|m| m.content.as_deref()),
            Some("message-39")
        );
    }

    #[test]
    fn repair_tool_call_message_chain_drops_incomplete_tool_turns() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: Some("run tool".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "call_orphan".to_string(),
                        call_type: "function".to_string(),
                        function: ChatToolCallFunction {
                            name: "memory_read".to_string(),
                            arguments: "{}".to_string(),
                        },
                    },
                ]),
            },
        ];
        let repaired = repair_tool_call_message_chain(messages);
        assert_eq!(repaired.len(), 1);
        assert_eq!(repaired[0].role, "user");
    }

    #[test]
    fn repair_tool_call_message_chain_drops_legacy_synthetic_tool_results() {
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: "memory_read".to_string(),
                        arguments: "{}".to_string(),
                    },
                }]),
            },
            ChatMessage {
                role: "tool".to_string(),
                content: Some(LEGACY_SYNTHETIC_TOOL_RESULT_UNAVAILABLE.to_string()),
                tool_call_id: Some("call_1".to_string()),
                name: None,
                tool_calls: None,
            },
        ];
        let repaired = repair_tool_call_message_chain(messages);
        assert!(repaired.is_empty());
    }

    #[test]
    fn repair_tool_call_message_chain_drops_turns_with_only_incomplete_tool_results() {
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: "memory_read".to_string(),
                        arguments: "{}".to_string(),
                    },
                }]),
            },
            ChatMessage {
                role: "tool".to_string(),
                content: Some(INCOMPLETE_TOOL_RESULT_MARK.to_string()),
                tool_call_id: Some("call_1".to_string()),
                name: Some("memory_read".to_string()),
                tool_calls: None,
            },
        ];
        let repaired = repair_tool_call_message_chain(messages);
        assert!(repaired.is_empty());
    }

    #[test]
    fn repair_tool_call_message_chain_drops_orphan_tool_messages() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: Some("hello".to_string()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: Some(r#"{"ok":false}"#.to_string()),
                tool_call_id: Some("call_orphan".to_string()),
                name: Some("den_capabilities_list_self".to_string()),
                tool_calls: None,
            },
        ];
        let repaired = repair_tool_call_message_chain(messages);
        assert_eq!(repaired.len(), 1);
        assert_eq!(repaired[0].role, "user");
    }

    #[test]
    fn reconstruct_transcript_defers_tool_result_until_tool_call() {
        let rows = vec![
            TranscriptRow {
                message_type: "user".to_string(),
                content_text: "list tools".to_string(),
                content_json: Value::Null,
            },
            TranscriptRow {
                message_type: "tool_result".to_string(),
                content_text: r#"{"ok":false}"#.to_string(),
                content_json: serde_json::json!({
                    "event": "tool_result",
                    "tool_call_id": "call_1",
                    "tool_name": "den_capabilities_list_self",
                    "status": "error",
                    "content": r#"{"ok":false}"#,
                }),
            },
            TranscriptRow {
                message_type: "tool_call".to_string(),
                content_text: "den_capabilities_list_self".to_string(),
                content_json: serde_json::json!({
                    "event": "tool_request",
                    "tool_call_id": "call_1",
                    "tool_name": "den_capabilities_list_self",
                    "args": {},
                }),
            },
        ];
        let messages = reconstruct_transcript_messages(rows);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert!(messages[1]
            .tool_calls
            .as_ref()
            .is_some_and(|calls| calls[0].id == "call_1"));
        assert_eq!(messages[2].role, "tool");
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn repair_tool_call_message_chain_keeps_complete_tool_turns() {
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: "memory_read".to_string(),
                        arguments: "{}".to_string(),
                    },
                }]),
            },
            ChatMessage {
                role: "tool".to_string(),
                content: Some(r#"{"ok":true}"#.to_string()),
                tool_call_id: Some("call_1".to_string()),
                name: Some("memory_read".to_string()),
                tool_calls: None,
            },
        ];
        let repaired = repair_tool_call_message_chain(messages);
        assert_eq!(repaired.len(), 2);
    }
}
