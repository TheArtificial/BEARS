use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::errors::CustomError;

#[derive(Debug, Clone)]
pub struct ConversationRecord {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub external_conversation_id: Option<String>,
    pub source_acp_session_id: Option<String>,
    pub current_title: Option<String>,
}

pub async fn ensure_conversation_for_external_id(
    pool: &PgPool,
    bear_id: Uuid,
    created_by_user_id: Option<i32>,
    external_conversation_id: &str,
    source_acp_session_id: Option<&str>,
    current_title: Option<&str>,
) -> Result<ConversationRecord, CustomError> {
    let row = sqlx::query(
        r#"
        INSERT INTO conversations (
            bear_id,
            created_by_user_id,
            external_conversation_id,
            source_acp_session_id,
            current_title
        )
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (bear_id, external_conversation_id)
        DO UPDATE SET
            source_acp_session_id = COALESCE(EXCLUDED.source_acp_session_id, conversations.source_acp_session_id),
            current_title = COALESCE(EXCLUDED.current_title, conversations.current_title),
            updated_at = NOW()
        RETURNING id, bear_id, external_conversation_id, source_acp_session_id, current_title
        "#,
    )
    .bind(bear_id)
    .bind(created_by_user_id)
    .bind(external_conversation_id)
    .bind(source_acp_session_id)
    .bind(current_title)
    .fetch_one(pool)
    .await
    .map_err(|err| CustomError::Database(format!("upsert conversation: {err}")))?;

    Ok(ConversationRecord {
        id: row
            .try_get("id")
            .map_err(|err| CustomError::Database(format!("decode conversation id: {err}")))?,
        bear_id: row
            .try_get("bear_id")
            .map_err(|err| CustomError::Database(format!("decode conversation bear_id: {err}")))?,
        external_conversation_id: row.try_get("external_conversation_id").map_err(|err| {
            CustomError::Database(format!("decode conversation external_conversation_id: {err}"))
        })?,
        source_acp_session_id: row.try_get("source_acp_session_id").map_err(|err| {
            CustomError::Database(format!("decode conversation source_acp_session_id: {err}"))
        })?,
        current_title: row.try_get("current_title").map_err(|err| {
            CustomError::Database(format!("decode conversation current_title: {err}"))
        })?,
    })
}

pub async fn insert_message_if_absent(
    pool: &PgPool,
    conversation_id: Uuid,
    sequence_no: i64,
    message_type: &str,
    role: Option<&str>,
    visibility: &str,
    content_text: &str,
    content_json: serde_json::Value,
    provider_message_id: Option<&str>,
    created_at: Option<&str>,
) -> Result<(), CustomError> {
    sqlx::query(
        r#"
        INSERT INTO conversation_messages (
            conversation_id,
            sequence_no,
            message_type,
            role,
            visibility,
            content_text,
            content_json,
            provider_message_id,
            created_at
        )
        VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8,
            COALESCE($9::timestamptz, NOW())
        )
        ON CONFLICT (conversation_id, sequence_no) DO NOTHING
        "#,
    )
    .bind(conversation_id)
    .bind(sequence_no)
    .bind(message_type)
    .bind(role)
    .bind(visibility)
    .bind(content_text)
    .bind(content_json)
    .bind(provider_message_id)
    .bind(created_at)
    .execute(pool)
    .await
    .map_err(|err| CustomError::Database(format!("insert conversation message: {err}")))?;
    Ok(())
}
