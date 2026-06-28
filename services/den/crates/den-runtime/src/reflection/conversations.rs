use sqlx::{PgPool, Row};
use time::Date;
use uuid::Uuid;

use den_service::conversation::persistence as conversation_persistence;
use den_core::DenError;

pub const MEMORY_CURATE_LANE: &str = "memory_curate";

#[derive(Debug, Clone)]
pub struct ReflectionConversationRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub role_agent_id: Option<String>,
    pub lane: String,
    pub conversation_date: Date,
    pub conversation_key: String,
    pub conversation_id: Option<String>,
    pub created_at: time::OffsetDateTime,
    pub last_used_at: time::OffsetDateTime,
}

pub fn memory_curate_conversation_key(conversation_date: Date) -> String {
    format!("memory_curate:{conversation_date}")
}

pub fn memory_curate_external_conversation_id(bear_id: Uuid, conversation_date: Date) -> String {
    format!(
        "conv-memory-curate-{}-{conversation_date}",
        bear_id.as_simple()
    )
}

pub async fn ensure_memory_curate_conversation(
    pool: &PgPool,
    bear_id: Uuid,
    role_agent_id: Option<&str>,
    conversation_date: Date,
) -> Result<ReflectionConversationRow, DenError> {
    let conversation_key = memory_curate_conversation_key(conversation_date);
    let external_conversation_id =
        memory_curate_external_conversation_id(bear_id, conversation_date);

    conversation_persistence::ensure_conversation_for_external_id(
        pool,
        bear_id,
        None,
        &external_conversation_id,
        None,
        Some(&format!("Memory curate {conversation_date}")),
    )
    .await?;

    let row = sqlx::query(
        r"
        INSERT INTO reflection_conversations (
            bear_id, role_agent_id, lane, conversation_date, conversation_key, conversation_id
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (bear_id, lane, conversation_date)
        DO UPDATE SET
            role_agent_id = COALESCE(EXCLUDED.role_agent_id, reflection_conversations.role_agent_id),
            conversation_key = EXCLUDED.conversation_key,
            conversation_id = COALESCE(reflection_conversations.conversation_id, EXCLUDED.conversation_id),
            last_used_at = NOW()
        RETURNING id, bear_id, role_agent_id, lane, conversation_date, conversation_key,
                  conversation_id, created_at, last_used_at
        ",
    )
    .bind(bear_id)
    .bind(role_agent_id)
    .bind(MEMORY_CURATE_LANE)
    .bind(conversation_date)
    .bind(&conversation_key)
    .bind(&external_conversation_id)
    .fetch_one(pool)
    .await?;

    Ok(row_from_sql(row))
}

pub async fn touch_memory_curate_conversation(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_date: Date,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE reflection_conversations
        SET last_used_at = NOW()
        WHERE bear_id = $1
          AND lane = $2
          AND conversation_date = $3
        ",
    )
    .bind(bear_id)
    .bind(MEMORY_CURATE_LANE)
    .bind(conversation_date)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn bind_memory_curate_run_conversation(
    pool: &PgPool,
    bear_id: Uuid,
    reflection_run_id: Uuid,
    conversation_id: &str,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE bear_reflection_runs
        SET conversation_id = $3
        WHERE bear_id = $1
          AND id = $2
          AND lane = $4
          AND (conversation_id IS NULL OR btrim(conversation_id) = '')
        ",
    )
    .bind(bear_id)
    .bind(reflection_run_id)
    .bind(conversation_id)
    .bind(MEMORY_CURATE_LANE)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_memory_curate_conversation(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_date: Date,
) -> Result<Option<ReflectionConversationRow>, DenError> {
    let row = sqlx::query(
        r"
        SELECT id, bear_id, role_agent_id, lane, conversation_date, conversation_key,
               conversation_id, created_at, last_used_at
        FROM reflection_conversations
        WHERE bear_id = $1
          AND lane = $2
          AND conversation_date = $3
        ",
    )
    .bind(bear_id)
    .bind(MEMORY_CURATE_LANE)
    .bind(conversation_date)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_from_sql))
}

fn row_from_sql(row: sqlx::postgres::PgRow) -> ReflectionConversationRow {
    ReflectionConversationRow {
        id: row.get("id"),
        bear_id: row.get("bear_id"),
        role_agent_id: row.get("role_agent_id"),
        lane: row.get("lane"),
        conversation_date: row.get("conversation_date"),
        conversation_key: row.get("conversation_key"),
        conversation_id: row.get("conversation_id"),
        created_at: row.get("created_at"),
        last_used_at: row.get("last_used_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_curate_external_conversation_id_is_conv_prefixed() {
        let bear_id = Uuid::new_v4();
        let date = Date::from_calendar_date(2026, time::Month::June, 8).expect("valid date");
        let external_id = memory_curate_external_conversation_id(bear_id, date);
        assert!(external_id.starts_with("conv-memory-curate-"));
    }
}
