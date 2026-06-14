use den_core::DenError;
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{
    conversation_message_types::{
        ConversationMessageRole, ConversationMessageType, ConversationMessageVisibility,
        ConversationMessageWrite,
    },
};

#[derive(Debug, Clone, Serialize)]
pub struct ConversationRecord {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub external_conversation_id: Option<String>,
    pub source_acp_session_id: Option<String>,
    pub current_title: Option<String>,
    pub updated_at: time::OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct PersistedConversationMessage {
    pub sequence_no: i64,
    pub message_type: String,
    pub role: Option<String>,
    pub visibility: String,
    pub content_text: String,
    pub provider_message_id: Option<String>,
    pub created_at: time::OffsetDateTime,
}

impl PersistedConversationMessage {
    pub fn storage_message_type(&self) -> Result<ConversationMessageType, DenError> {
        ConversationMessageType::try_from_storage(&self.message_type)
    }

    pub fn storage_visibility(&self) -> Result<ConversationMessageVisibility, DenError> {
        ConversationMessageVisibility::try_from_storage(&self.visibility)
    }

    pub fn storage_role(&self) -> Result<Option<ConversationMessageRole>, DenError> {
        match self.role.as_deref() {
            None => Ok(None),
            Some(role) => ConversationMessageRole::try_from_storage(role).map(Some),
        }
    }

    /// Rows that should appear in user-facing chat history (web chat / pair reflection).
    pub fn is_transcript_visible(&self) -> bool {
        let Ok(visibility) = self.storage_visibility() else {
            return false;
        };
        if !visibility.is_transcript_visible() {
            return false;
        }
        matches!(
            self.role.as_deref(),
            Some("user") | Some("assistant")
        )
    }
}

pub async fn ensure_conversation_for_external_id(
    pool: &PgPool,
    bear_id: Uuid,
    created_by_user_id: Option<i32>,
    external_conversation_id: &str,
    source_acp_session_id: Option<&str>,
    current_title: Option<&str>,
) -> Result<ConversationRecord, DenError> {
    let inserted_row = sqlx::query(
        r#"
        INSERT INTO conversations (
            bear_id,
            created_by_user_id,
            external_conversation_id,
            source_acp_session_id,
            current_title
        )
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT DO NOTHING
        RETURNING id, bear_id, external_conversation_id, source_acp_session_id, current_title, updated_at
        "#,
    )
    .bind(bear_id)
    .bind(created_by_user_id)
    .bind(external_conversation_id)
    .bind(source_acp_session_id)
    .bind(current_title)
    .fetch_optional(pool)
    .await
    .map_err(|err| DenError::Database(format!("upsert conversation insert: {err}")))?;

    let row = if let Some(row) = inserted_row {
        row
    } else {
        sqlx::query(
            r#"
            UPDATE conversations
            SET source_acp_session_id = COALESCE($3, conversations.source_acp_session_id),
                current_title = COALESCE($4, conversations.current_title)
            WHERE bear_id = $1
              AND external_conversation_id = $2
            RETURNING id, bear_id, external_conversation_id, source_acp_session_id, current_title, updated_at
            "#,
        )
        .bind(bear_id)
        .bind(external_conversation_id)
        .bind(source_acp_session_id)
        .bind(current_title)
        .fetch_one(pool)
        .await
        .map_err(|err| DenError::Database(format!("upsert conversation update: {err}")))?
    };

    Ok(ConversationRecord {
        id: row
            .try_get("id")
            .map_err(|err| DenError::Database(format!("decode conversation id: {err}")))?,
        bear_id: row
            .try_get("bear_id")
            .map_err(|err| DenError::Database(format!("decode conversation bear_id: {err}")))?,
        external_conversation_id: row.try_get("external_conversation_id").map_err(|err| {
            DenError::Database(format!("decode conversation external_conversation_id: {err}"))
        })?,
        source_acp_session_id: row.try_get("source_acp_session_id").map_err(|err| {
            DenError::Database(format!("decode conversation source_acp_session_id: {err}"))
        })?,
        current_title: row.try_get("current_title").map_err(|err| {
            DenError::Database(format!("decode conversation current_title: {err}"))
        })?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|err| DenError::Database(format!("decode conversation updated_at: {err}")))?,
    })
}

pub async fn get_conversation_by_id(
    pool: &PgPool,
    conversation_id: Uuid,
) -> Result<Option<ConversationRecord>, DenError> {
    let row = sqlx::query(
        r#"
        SELECT id, bear_id, external_conversation_id, source_acp_session_id, current_title, updated_at
        FROM conversations
        WHERE id = $1
        LIMIT 1
        "#,
    )
    .bind(conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| DenError::Database(format!("get conversation by id: {err}")))?;

    row.map(|row| {
        Ok(ConversationRecord {
            id: row.try_get("id").map_err(|err| {
                DenError::Database(format!("decode conversation id: {err}"))
            })?,
            bear_id: row.try_get("bear_id").map_err(|err| {
                DenError::Database(format!("decode conversation bear_id: {err}"))
            })?,
            external_conversation_id: row.try_get("external_conversation_id").map_err(|err| {
                DenError::Database(format!("decode conversation external_conversation_id: {err}"))
            })?,
            source_acp_session_id: row.try_get("source_acp_session_id").map_err(|err| {
                DenError::Database(format!("decode conversation source_acp_session_id: {err}"))
            })?,
            current_title: row.try_get("current_title").map_err(|err| {
                DenError::Database(format!("decode conversation current_title: {err}"))
            })?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|err| DenError::Database(format!("decode conversation updated_at: {err}")))?,
        })
    })
    .transpose()
}

pub async fn get_conversation_for_external_id(
    pool: &PgPool,
    bear_id: Uuid,
    external_conversation_id: &str,
) -> Result<Option<ConversationRecord>, DenError> {
    let row = sqlx::query(
        r#"
        SELECT id, bear_id, external_conversation_id, source_acp_session_id, current_title, updated_at
        FROM conversations
        WHERE bear_id = $1
          AND external_conversation_id = $2
        LIMIT 1
        "#,
    )
    .bind(bear_id)
    .bind(external_conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| DenError::Database(format!("get conversation by external id: {err}")))?;

    row.map(|row| {
        Ok(ConversationRecord {
            id: row.try_get("id").map_err(|err| {
                DenError::Database(format!("decode conversation id: {err}"))
            })?,
            bear_id: row.try_get("bear_id").map_err(|err| {
                DenError::Database(format!("decode conversation bear_id: {err}"))
            })?,
            external_conversation_id: row.try_get("external_conversation_id").map_err(|err| {
                DenError::Database(format!("decode conversation external_conversation_id: {err}"))
            })?,
            source_acp_session_id: row.try_get("source_acp_session_id").map_err(|err| {
                DenError::Database(format!("decode conversation source_acp_session_id: {err}"))
            })?,
            current_title: row.try_get("current_title").map_err(|err| {
                DenError::Database(format!("decode conversation current_title: {err}"))
            })?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|err| DenError::Database(format!("decode conversation updated_at: {err}")))?,
        })
    })
    .transpose()
}

pub async fn delete_conversation_for_external_id(
    pool: &PgPool,
    bear_id: Uuid,
    external_conversation_id: &str,
) -> Result<u64, DenError> {
    let result = sqlx::query(
        r#"
        DELETE FROM conversations
        WHERE bear_id = $1
          AND external_conversation_id = $2
        "#,
    )
    .bind(bear_id)
    .bind(external_conversation_id)
    .execute(pool)
    .await
    .map_err(|err| DenError::Database(format!("delete conversation by external id: {err}")))?;
    Ok(result.rows_affected())
}

pub async fn set_conversation_title(
    pool: &PgPool,
    bear_id: Uuid,
    external_conversation_id: &str,
    title: &str,
) -> Result<u64, DenError> {
    let normalized = title.trim();
    if normalized.is_empty() {
        return Ok(0);
    }
    let result = sqlx::query(
        r#"
        UPDATE conversations
        SET current_title = $3,
            updated_at = NOW()
        WHERE bear_id = $1
          AND external_conversation_id = $2
        "#,
    )
    .bind(bear_id)
    .bind(external_conversation_id)
    .bind(normalized)
    .execute(pool)
    .await
    .map_err(|err| DenError::Database(format!("update conversation title: {err}")))?;
    Ok(result.rows_affected())
}

pub async fn list_conversations_for_bear(
    pool: &PgPool,
    bear_id: Uuid,
    limit: i64,
) -> Result<Vec<ConversationRecord>, DenError> {
    let rows = sqlx::query(
        r#"
        SELECT id, bear_id, external_conversation_id, source_acp_session_id, current_title, updated_at
        FROM conversations
        WHERE bear_id = $1
        ORDER BY updated_at DESC
        LIMIT $2
        "#,
    )
    .bind(bear_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await
    .map_err(|err| DenError::Database(format!("list conversations for bear: {err}")))?;

    rows.into_iter()
        .map(|row| {
            Ok(ConversationRecord {
                id: row.try_get("id").map_err(|err| {
                    DenError::Database(format!("decode conversation id: {err}"))
                })?,
                bear_id: row.try_get("bear_id").map_err(|err| {
                    DenError::Database(format!("decode conversation bear_id: {err}"))
                })?,
                external_conversation_id: row.try_get("external_conversation_id").map_err(|err| {
                    DenError::Database(format!("decode conversation external_conversation_id: {err}"))
                })?,
                source_acp_session_id: row.try_get("source_acp_session_id").map_err(|err| {
                    DenError::Database(format!("decode conversation source_acp_session_id: {err}"))
                })?,
                current_title: row.try_get("current_title").map_err(|err| {
                    DenError::Database(format!("decode conversation current_title: {err}"))
                })?,
                updated_at: row.try_get("updated_at").map_err(|err| {
                    DenError::Database(format!("decode conversation updated_at: {err}"))
                })?,
            })
        })
        .collect()
}

pub async fn list_messages_page(
    pool: &PgPool,
    conversation_id: Uuid,
    before_sequence_no: Option<i64>,
    limit: i64,
) -> Result<Vec<PersistedConversationMessage>, DenError> {
    let rows = sqlx::query(
        r#"
        SELECT sequence_no, message_type, role, visibility, content_text, provider_message_id, created_at
        FROM conversation_messages
        WHERE conversation_id = $1
          AND ($2::bigint IS NULL OR sequence_no < $2)
        ORDER BY sequence_no DESC
        LIMIT $3
        "#,
    )
    .bind(conversation_id)
    .bind(before_sequence_no)
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await
    .map_err(|err| DenError::Database(format!("list conversation messages: {err}")))?;

    rows.into_iter()
        .map(|row| {
            Ok(PersistedConversationMessage {
                sequence_no: row.try_get("sequence_no").map_err(|err| {
                    DenError::Database(format!("decode conversation message sequence_no: {err}"))
                })?,
                message_type: row.try_get("message_type").map_err(|err| {
                    DenError::Database(format!("decode conversation message message_type: {err}"))
                })?,
                role: row.try_get("role").map_err(|err| {
                    DenError::Database(format!("decode conversation message role: {err}"))
                })?,
                visibility: row.try_get("visibility").map_err(|err| {
                    DenError::Database(format!("decode conversation message visibility: {err}"))
                })?,
                content_text: row.try_get("content_text").map_err(|err| {
                    DenError::Database(format!("decode conversation message content_text: {err}"))
                })?,
                provider_message_id: row.try_get("provider_message_id").map_err(|err| {
                    DenError::Database(format!("decode conversation message provider_message_id: {err}"))
                })?,
                created_at: row.try_get("created_at").map_err(|err| {
                    DenError::Database(format!("decode conversation message created_at: {err}"))
                })?,
            })
        })
        .collect()
}

pub async fn append_message(
    pool: &PgPool,
    conversation_id: Uuid,
    message: &ConversationMessageWrite,
) -> Result<i64, DenError> {
    let message_type = message.message_type.as_str();
    let role = message.role.map(|r| r.as_str());
    let visibility = message.visibility.as_str();
    let content_text = message.content_text.as_str();
    let content_json = message.content_json.clone();
    let provider_message_id = message.provider_message_id.as_deref();
    let source_event_id = message.source_event_id.as_deref();
    let created_at = message.created_at.as_deref();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| DenError::Database(format!("begin append conversation message tx: {err}")))?;

    if let Some(source_event_id) = source_event_id {
        if let Some(existing_sequence_no) = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT sequence_no
            FROM conversation_messages
            WHERE conversation_id = $1
              AND source_event_id = $2
            LIMIT 1
            "#,
        )
        .bind(conversation_id)
        .bind(source_event_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| DenError::Database(format!("lookup conversation message source_event_id: {err}")))?
        {
            tx.rollback().await.map_err(|err| {
                DenError::Database(format!("rollback append conversation message tx: {err}"))
            })?;
            return Ok(existing_sequence_no);
        }
    }

    let allocator_row = sqlx::query(
        r#"
        UPDATE conversations
        SET next_message_sequence = next_message_sequence + 1,
            updated_at = NOW()
        WHERE id = $1
        RETURNING next_message_sequence - 1 AS sequence_no
        "#,
    )
    .bind(conversation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| DenError::Database(format!("allocate conversation message sequence: {err}")))?;

    let sequence_no: i64 = allocator_row
        .try_get("sequence_no")
        .map_err(|err| DenError::Database(format!("decode allocated sequence_no: {err}")))?;

    if let Err(err) = sqlx::query(
        r#"
        INSERT INTO conversation_messages (
            conversation_id,
            sequence_no,
            message_type,
            role,
            visibility,
            content_text,
            content_json,
            source_event_id,
            provider_message_id,
            created_at
        )
        VALUES (
            $1,
            $2,
            $3,
            $4,
            $5,
            $6,
            $7,
            $8,
            $9,
            COALESCE($10::timestamptz, NOW())
        )
        "#,
    )
    .bind(conversation_id)
    .bind(sequence_no)
    .bind(message_type)
    .bind(role)
    .bind(visibility)
    .bind(content_text)
    .bind(content_json)
    .bind(source_event_id)
    .bind(provider_message_id)
    .bind(created_at)
    .execute(&mut *tx)
    .await
    {
        tx.rollback().await.map_err(|rollback_err| {
            DenError::Database(format!("rollback append conversation message tx: {rollback_err}"))
        })?;

        let duplicate_sequence_no = if source_event_id.is_some() {
            sqlx::query_scalar::<_, i64>(
                r#"
                SELECT sequence_no
                FROM conversation_messages
                WHERE conversation_id = $1
                  AND source_event_id = $2
                LIMIT 1
                "#,
            )
            .bind(conversation_id)
            .bind(source_event_id)
            .fetch_optional(pool)
            .await
            .map_err(|reload_err| {
                DenError::Database(format!(
                    "reload duplicate conversation message sequence after insert error: {reload_err}"
                ))
            })?
        } else {
            None
        };
        if let Some(existing_sequence_no) = duplicate_sequence_no {
            return Ok(existing_sequence_no);
        }
        return Err(DenError::Database(format!(
            "append conversation message: {err}"
        )));
    }

    tx.commit()
        .await
        .map_err(|err| DenError::Database(format!("commit append conversation message tx: {err}")))?;

    Ok(sequence_no)
}

pub async fn insert_message_if_absent(
    pool: &PgPool,
    conversation_id: Uuid,
    sequence_no: i64,
    message: &ConversationMessageWrite,
) -> Result<(), DenError> {
    let message_type = message.message_type.as_str();
    let role = message.role.map(|r| r.as_str());
    let visibility = message.visibility.as_str();
    let content_text = message.content_text.as_str();
    let content_json = message.content_json.clone();
    let provider_message_id = message.provider_message_id.as_deref();
    let created_at = message.created_at.as_deref();
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
    .map_err(|err| DenError::Database(format!("insert conversation message: {err}")))?;
    Ok(())
}

pub async fn count_visible_messages(
    pool: &PgPool,
    conversation_id: Uuid,
) -> Result<i64, DenError> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::bigint
        FROM conversation_messages
        WHERE conversation_id = $1
          AND visibility != 'diagnostic_only'
        "#,
    )
    .bind(conversation_id)
    .fetch_one(pool)
    .await
    .map_err(|err| DenError::Database(format!("count visible conversation messages: {err}")))
}

