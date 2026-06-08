use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    core::llm::ChatMessage,
    errors::CustomError,
};

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
        });
    }
    let history_rows = sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT role, content_text
        FROM conversation_messages
        WHERE conversation_id = (
            SELECT id FROM conversations
            WHERE external_conversation_id = $1 AND bear_id = $2
            LIMIT 1
        )
        AND visibility = 'visible'
        ORDER BY sequence_no ASC
        LIMIT 40
        "#,
    )
    .bind(conversation_id)
    .bind(bear_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (role, content) in history_rows {
        if content.trim().is_empty() {
            continue;
        }
        messages.push(ChatMessage {
            role,
            content: Some(content),
            tool_call_id: None,
            name: None,
        });
    }
    if let Some(human) = human_message.filter(|s| !s.is_empty()) {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: Some(human.to_string()),
            tool_call_id: None,
            name: None,
        });
    }
    messages.extend(tool_messages.iter().cloned());
    let _ = pool;
    Ok(messages)
}
