use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row as SqlxRow};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertClientSession {
    pub user_id: i32,
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub client_session_id: String,
    pub runtime_session_id: String,
    pub conversation_id: String,
    pub resolved_conversation_id: Option<String>,
    pub client: String,
    pub cwd: Option<String>,
    #[serde(default)]
    pub current_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ClientSessionRow {
    pub id: Uuid,
    pub user_id: i32,
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub client_session_id: String,
    pub runtime_session_id: String,
    pub conversation_id: String,
    pub resolved_conversation_id: Option<String>,
    pub client: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_environment: Option<serde_json::Value>,
    pub current_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_title_updated_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_title_synced_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedWorkspaceContext {
    pub cwd: Option<String>,
    pub roots: Vec<String>,
    pub source: String,
}

impl ClientSessionRow {
    pub fn trusted_workspace_context(&self) -> TrustedWorkspaceContext {
        let roots = self
            .adapter_environment
            .as_ref()
            .and_then(|value| value.get("workspace_roots"))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cwd = self
            .adapter_environment
            .as_ref()
            .and_then(|value| value.get("cwd"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                self.cwd
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            });
        let roots = if roots.is_empty() {
            cwd.as_ref()
                .map(|cwd| vec![cwd.clone()])
                .unwrap_or_default()
        } else {
            roots
        };
        let source = if !roots.is_empty() || cwd.is_some() {
            "trusted_session".to_string()
        } else {
            "none".to_string()
        };
        TrustedWorkspaceContext { cwd, roots, source }
    }
}

const UPSERT_SESSION_SQL: &str = r"
        INSERT INTO client_sessions (
            user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
            conversation_id, resolved_conversation_id, client, cwd, current_mode
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, COALESCE($10, 'ask'))
        ON CONFLICT (user_id, bear_id, client_session_id) DO UPDATE
        SET bear_slug = EXCLUDED.bear_slug,
            runtime_session_id = EXCLUDED.runtime_session_id,
            conversation_id = COALESCE(NULLIF(EXCLUDED.conversation_id, ''), client_sessions.conversation_id),
            resolved_conversation_id = COALESCE(EXCLUDED.resolved_conversation_id, client_sessions.resolved_conversation_id),
            client = EXCLUDED.client,
            cwd = COALESCE(EXCLUDED.cwd, client_sessions.cwd),
            current_mode = COALESCE(client_sessions.current_mode, EXCLUDED.current_mode),
            closed_at = NULL,
            archived_at = NULL,
            updated_at = NOW()
        ";

pub async fn upsert_session(pool: &PgPool, session: UpsertClientSession) -> Result<(), DenError> {
    sqlx::query(UPSERT_SESSION_SQL)
        .bind(session.user_id)
        .bind(session.bear_id)
        .bind(session.bear_slug)
        .bind(session.client_session_id)
        .bind(session.runtime_session_id)
        .bind(session.conversation_id)
        .bind(session.resolved_conversation_id)
        .bind(session.client)
        .bind(session.cwd)
        .bind(session.current_mode)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_current_mode(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    mode: &str,
) -> Result<(), DenError> {
    if !matches!(mode, "ask" | "plan" | "write") {
        return Err(DenError::ValidationError(
            "client session mode must be one of ask, plan, write".to_string(),
        ));
    }
    sqlx::query(
        r"
        UPDATE client_sessions
        SET current_mode = $4, updated_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
    )
    .bind(user_id)
    .bind(bear_id)
    .bind(client_session_id)
    .bind(mode)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_resolved(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    resolved_conversation_id: &str,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE client_sessions
        SET resolved_conversation_id = $4, updated_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
    )
    .bind(user_id)
    .bind(bear_id)
    .bind(client_session_id)
    .bind(resolved_conversation_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_for_user_bear_session(
    pool: &PgPool,
    user_id: i32,
    bear_slug: &str,
    client_session_id: &str,
) -> Result<Option<ClientSessionRow>, DenError> {
    let row = sqlx::query_as::<_, ClientSessionRow>(
        r"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
               conversation_id, resolved_conversation_id, client, cwd, adapter_environment, current_mode,
               conversation_title, conversation_title_updated_at, conversation_title_synced_at,
               closed_at, archived_at, created_at, updated_at
        FROM client_sessions
        WHERE user_id = $1 AND bear_slug = $2 AND client_session_id = $3
        ",
    )
    .bind(user_id)
    .bind(bear_slug)
    .bind(client_session_id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

pub async fn find_for_user_bear_session_id(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
) -> Result<Option<ClientSessionRow>, DenError> {
    let row = sqlx::query_as::<_, ClientSessionRow>(
        r"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
               conversation_id, resolved_conversation_id, client, cwd, adapter_environment, current_mode,
               conversation_title, conversation_title_updated_at, conversation_title_synced_at,
               closed_at, archived_at, created_at, updated_at
        FROM client_sessions
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
    )
    .bind(user_id)
    .bind(bear_id)
    .bind(client_session_id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Lists persisted client sessions for a user on a bear, newest activity first.
pub struct SessionListParams<'a> {
    pub user_id: i32,
    pub bear_slug: &'a str,
    pub include_closed: bool,
    pub cwd_filter: Option<&'a str>,
    pub limit: i64,
    pub cursor_updated_at: Option<OffsetDateTime>,
    pub cursor_id: Option<Uuid>,
}

pub async fn list_for_user_bear(
    pool: &PgPool,
    params: SessionListParams<'_>,
) -> Result<Vec<ClientSessionRow>, DenError> {
    let limit = params.limit.clamp(1, 100);
    let cwd_filter = params.cwd_filter.map(str::trim).filter(|s| !s.is_empty());
    let rows = sqlx::query_as::<_, ClientSessionRow>(
        r"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
               conversation_id, resolved_conversation_id, client, cwd, adapter_environment, current_mode,
               conversation_title, conversation_title_updated_at, conversation_title_synced_at,
               closed_at, archived_at, created_at, updated_at
        FROM client_sessions
        WHERE user_id = $1 AND bear_slug = $2
          AND ($3 OR closed_at IS NULL)
          AND ($4::text IS NULL OR cwd IS NOT DISTINCT FROM $4)
          AND (
            $6::timestamptz IS NULL
            OR updated_at < $6
            OR (updated_at = $6 AND id < $7)
          )
        ORDER BY updated_at DESC, id DESC
        LIMIT $5
        ",
    )
    .bind(params.user_id)
    .bind(params.bear_slug)
    .bind(params.include_closed)
    .bind(cwd_filter)
    .bind(limit)
    .bind(params.cursor_updated_at)
    .bind(params.cursor_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn update_adapter_environment(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    adapter_environment: &serde_json::Value,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE client_sessions
        SET adapter_environment = $4,
            updated_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
    )
    .bind(user_id)
    .bind(bear_id)
    .bind(client_session_id)
    .bind(adapter_environment)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_client_conversation_title(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    title: Option<&str>,
) -> Result<(), DenError> {
    let normalized = title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(120).collect::<String>());
    sqlx::query(
        r"
        UPDATE client_sessions
        SET conversation_title = $4,
            conversation_title_updated_at = CASE
                WHEN $4::text IS NULL THEN conversation_title_updated_at
                WHEN conversation_title IS DISTINCT FROM $4 THEN NOW()
                ELSE conversation_title_updated_at
            END,
            conversation_title_synced_at = CASE
                WHEN $4::text IS NULL THEN conversation_title_synced_at
                WHEN conversation_title IS DISTINCT FROM $4 THEN NULL
                ELSE conversation_title_synced_at
            END,
            updated_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
    )
    .bind(user_id)
    .bind(bear_id)
    .bind(client_session_id)
    .bind(normalized)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_closed(pool: &PgPool, id: Uuid) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE client_sessions
        SET closed_at = NOW(), updated_at = NOW()
        WHERE id = $1
        ",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_title_for_bear_conversation(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    title: &str,
) -> Result<u64, DenError> {
    let result = sqlx::query(
        r"
        UPDATE client_sessions
        SET conversation_title = $3,
            conversation_title_updated_at = NOW(),
            conversation_title_synced_at = NULL,
            updated_at = NOW()
        WHERE bear_id = $1
          AND (conversation_id = $2 OR resolved_conversation_id = $2)
        ",
    )
    .bind(bear_id)
    .bind(conversation_id)
    .bind(title)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn mark_title_synced(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE client_sessions
        SET conversation_title_synced_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
    )
    .bind(user_id)
    .bind(bear_id)
    .bind(client_session_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn resolved_conversation_ids_for_bear(
    pool: &PgPool,
    bear_slug: &str,
) -> Result<Vec<String>, DenError> {
    let rows = sqlx::query(
        r"
        SELECT DISTINCT resolved_conversation_id
        FROM client_sessions
        WHERE bear_slug = $1
          AND resolved_conversation_id IS NOT NULL
          AND resolved_conversation_id LIKE 'conv-%'
        ",
    )
    .bind(bear_slug)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .filter_map(|row| row.get::<Option<String>, _>("resolved_conversation_id"))
        .collect())
}

pub async fn mark_archived(pool: &PgPool, id: Uuid) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE client_sessions
        SET archived_at = NOW(), updated_at = NOW()
        WHERE id = $1
        ",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
