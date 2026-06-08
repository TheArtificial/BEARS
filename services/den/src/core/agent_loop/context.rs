use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    core::llm::ChatMessage,
    errors::CustomError,
};

pub async fn load_transcript_messages(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
) -> Result<Vec<ChatMessage>, CustomError> {
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
    Ok(history_rows
        .into_iter()
        .filter(|(_, content)| !content.trim().is_empty())
        .map(|(role, content)| ChatMessage {
            role,
            content: Some(content),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        })
        .collect())
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
