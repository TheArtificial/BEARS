use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientSessionMode {
    Ask,
    Plan,
    Write,
}

impl ClientSessionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Plan => "plan",
            Self::Write => "write",
        }
    }

    pub fn try_from_storage(value: &str) -> Result<Self, DenError> {
        match value {
            "ask" => Ok(Self::Ask),
            "plan" => Ok(Self::Plan),
            "write" => Ok(Self::Write),
            other => Err(DenError::ValidationError(format!(
                "unsupported client session mode: {other}"
            ))),
        }
    }
}

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
    pub current_mode: Option<ClientSessionMode>,
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
    /// The Docket task explicitly selected as this session's current objective.
    /// This is distinct from an active Docket execution and from a Work assignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<Uuid>,
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

pub async fn upsert_session(pool: &PgPool, session: UpsertClientSession) -> Result<(), DenError> {
    sqlx::query!(
        r"
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
        ",
        session.user_id,
        session.bear_id,
        session.bear_slug,
        session.client_session_id,
        session.runtime_session_id,
        session.conversation_id,
        session.resolved_conversation_id,
        session.client,
        session.cwd,
        session.current_mode.map(ClientSessionMode::as_str),
    )
    .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_current_mode(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    mode: ClientSessionMode,
) -> Result<(), DenError> {
    sqlx::query!(
        r"
        UPDATE client_sessions
        SET current_mode = $4, updated_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
        user_id,
        bear_id,
        client_session_id,
        mode.as_str()
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_current_task(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    task_id: Option<Uuid>,
) -> Result<(), DenError> {
    sqlx::query!(
        r"
        UPDATE client_sessions
        SET current_task_id = $4, updated_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
        user_id,
        bear_id,
        client_session_id,
        task_id
    )
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
    sqlx::query!(
        r"
        UPDATE client_sessions
        SET resolved_conversation_id = $4, updated_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
        user_id,
        bear_id,
        client_session_id,
        resolved_conversation_id
    )
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
    let row = sqlx::query_as!(ClientSessionRow,
        r"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
               conversation_id, resolved_conversation_id, client, cwd, adapter_environment, current_mode, current_task_id,
               conversation_title, conversation_title_updated_at, conversation_title_synced_at,
               closed_at, archived_at, created_at, updated_at
        FROM client_sessions
        WHERE user_id = $1 AND bear_slug = $2 AND client_session_id = $3
        ",
    user_id, bear_slug, client_session_id)
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
    let row = sqlx::query_as!(ClientSessionRow,
        r"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
               conversation_id, resolved_conversation_id, client, cwd, adapter_environment, current_mode, current_task_id,
               conversation_title, conversation_title_updated_at, conversation_title_synced_at,
               closed_at, archived_at, created_at, updated_at
        FROM client_sessions
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
    user_id, bear_id, client_session_id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

pub async fn find_latest_for_bear_conversation(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
) -> Result<Option<ClientSessionRow>, DenError> {
    let row = sqlx::query_as!(ClientSessionRow,
        r"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
               conversation_id, resolved_conversation_id, client, cwd, adapter_environment, current_mode, current_task_id,
               conversation_title, conversation_title_updated_at, conversation_title_synced_at,
               closed_at, archived_at, created_at, updated_at
        FROM client_sessions
        WHERE bear_id = $1
          AND (conversation_id = $2 OR resolved_conversation_id = $2)
        ORDER BY updated_at DESC, id DESC
        LIMIT 1
        ",
    bear_id, conversation_id)
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

#[derive(Debug, Clone)]
pub struct OpenReflectionCandidatesParams {
    pub stale_after_minutes: i64,
    pub activity_threshold: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OpenReflectionCandidateRow {
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
    /// The Docket task explicitly selected as this session's current objective.
    /// This is distinct from an active Docket execution and from a Work assignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<Uuid>,
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
    pub event_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reflected_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_compaction_source_end_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reflected_source_end_seq: Option<i64>,
    pub reflection_trigger: String,
}

impl OpenReflectionCandidateRow {
    pub fn session(&self) -> ClientSessionRow {
        ClientSessionRow {
            id: self.id,
            user_id: self.user_id,
            bear_id: self.bear_id,
            bear_slug: self.bear_slug.clone(),
            client_session_id: self.client_session_id.clone(),
            runtime_session_id: self.runtime_session_id.clone(),
            conversation_id: self.conversation_id.clone(),
            resolved_conversation_id: self.resolved_conversation_id.clone(),
            client: self.client.clone(),
            cwd: self.cwd.clone(),
            adapter_environment: self.adapter_environment.clone(),
            current_mode: self.current_mode.clone(),
            current_task_id: self.current_task_id,
            conversation_title: self.conversation_title.clone(),
            conversation_title_updated_at: self.conversation_title_updated_at,
            conversation_title_synced_at: self.conversation_title_synced_at,
            closed_at: self.closed_at,
            archived_at: self.archived_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Finds open sessions eligible for automatic pair reflection.
///
/// ponytail: activity eligibility probes only through the configured threshold;
/// exact event counts are collected only for the selected sweep candidates.
/// The ceiling is a separate indexed event probe per open session; persist
/// reflection state on the session if the open-session population grows large.
pub async fn list_open_reflection_candidates(
    pool: &PgPool,
    params: OpenReflectionCandidatesParams,
) -> Result<Vec<OpenReflectionCandidateRow>, DenError> {
    let default_stale_after_minutes = params.stale_after_minutes.max(1);
    let default_activity_threshold = params.activity_threshold.max(1);
    let default_limit = params.limit.clamp(1, 100);
    let rows = sqlx::query_as!(
        OpenReflectionCandidateRow,
        r#"
        WITH open_sessions AS (
            SELECT s.*,
                   reflected.last_reflected_at,
                   latest_compaction.source_message_end_seq AS latest_compaction_source_end_seq,
                   reflected.last_reflected_source_end_seq,
                   COALESCE(b.live_reflection_stale_after_minutes::bigint, $1::bigint) AS stale_after_minutes,
                   COALESCE(b.live_reflection_activity_threshold::bigint, $2::bigint) AS activity_threshold,
                   COALESCE(b.live_reflection_sweep_limit::bigint, $3::bigint) AS sweep_limit,
                   CASE
                       WHEN s.updated_at <= NOW() - (COALESCE(b.live_reflection_stale_after_minutes::bigint, $1::bigint) * INTERVAL '1 minute') THEN 'stale_open_sweep'
                       ELSE 'activity_threshold_sweep'
                   END AS reflection_trigger,
                   CASE
                       WHEN s.updated_at <= NOW() - (COALESCE(b.live_reflection_stale_after_minutes::bigint, $1::bigint) * INTERVAL '1 minute') THEN FALSE
                       ELSE EXISTS (
                           SELECT 1
                           FROM bearwire_events e
                           WHERE e.session_id = s.client_session_id
                             AND e.bear_id = s.bear_id
                             AND e.user_id = s.user_id
                           OFFSET (GREATEST(COALESCE(b.live_reflection_activity_threshold::bigint, $2::bigint), 1) - 1)
                           LIMIT 1
                       )
                   END AS activity_threshold_reached
            FROM client_sessions s
            INNER JOIN bears b ON b.id = s.bear_id
            LEFT JOIN LATERAL (
                SELECT MAX(e.created_at) AS reflection_requeued_at
                FROM bearwire_events e
                WHERE e.session_id = s.client_session_id
                  AND e.bear_id = s.bear_id
                  AND e.user_id = s.user_id
                  AND e.event_type = 'session.reflection_requeued'
            ) requeued ON TRUE
            LEFT JOIN LATERAL (
                SELECT MAX(e.created_at) AS last_reflected_at,
                       MAX(
                           CASE
                               WHEN e.event_json #>> '{data,pair_reflection,status}' = 'processed'
                                AND e.event_json #>> '{data,pair_reflection,source_message_end_seq}' ~ '^[0-9]+$'
                                   THEN (e.event_json #>> '{data,pair_reflection,source_message_end_seq}')::bigint
                               ELSE NULL
                           END
                       ) AS last_reflected_source_end_seq
                FROM bearwire_events e
                WHERE e.session_id = s.client_session_id
                  AND e.bear_id = s.bear_id
                  AND e.user_id = s.user_id
                  AND e.event_type = 'session.reflected'
                  AND e.created_at > COALESCE(requeued.reflection_requeued_at, '-infinity'::timestamptz)
            ) reflected ON TRUE
            LEFT JOIN LATERAL (
                SELECT a.source_message_end_seq
                FROM conversations c
                INNER JOIN conversation_compaction_artifacts a ON a.conversation_id = c.id
                WHERE c.bear_id = s.bear_id
                  AND c.external_conversation_id = COALESCE(NULLIF(s.resolved_conversation_id, ''), s.conversation_id)
                  AND a.artifact_kind = 'iterative_summary'
                  AND a.superseded_by IS NULL
                ORDER BY a.created_at DESC
                LIMIT 1
            ) latest_compaction ON TRUE
            WHERE s.closed_at IS NULL
              AND s.archived_at IS NULL
              AND b.live_reflection_enabled IS TRUE
        ), eligible_sessions AS (
            SELECT *,
                   ROW_NUMBER() OVER (PARTITION BY bear_id ORDER BY updated_at ASC, id ASC) AS bear_sweep_rank
            FROM open_sessions
            WHERE (updated_at <= NOW() - (stale_after_minutes * INTERVAL '1 minute')
                   OR activity_threshold_reached)
              AND (
                  (last_reflected_at IS NULL AND last_reflected_source_end_seq IS NULL)
                  OR latest_compaction_source_end_seq > COALESCE(last_reflected_source_end_seq, 0)
              )
        ), selected_sessions AS MATERIALIZED (
            SELECT *
            FROM eligible_sessions
            WHERE bear_sweep_rank <= sweep_limit
            ORDER BY updated_at ASC, id ASC
            LIMIT $3
        )
        SELECT id, user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
               conversation_id, resolved_conversation_id, client, cwd, adapter_environment, current_mode, current_task_id,
               conversation_title, conversation_title_updated_at, conversation_title_synced_at,
               closed_at, archived_at, created_at, updated_at,
               events.event_count AS "event_count!", last_reflected_at,
               latest_compaction_source_end_seq, last_reflected_source_end_seq,
               reflection_trigger AS "reflection_trigger!"
        FROM selected_sessions
        LEFT JOIN LATERAL (
            SELECT COUNT(*)::bigint AS event_count
            FROM bearwire_events e
            WHERE e.session_id = selected_sessions.client_session_id
              AND e.bear_id = selected_sessions.bear_id
              AND e.user_id = selected_sessions.user_id
        ) events ON TRUE
        ORDER BY updated_at ASC, id ASC
        "#,
        default_stale_after_minutes,
        default_activity_threshold,
        default_limit,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn list_for_user_bear(
    pool: &PgPool,
    params: SessionListParams<'_>,
) -> Result<Vec<ClientSessionRow>, DenError> {
    let limit = params.limit.clamp(1, 100);
    let cwd_filter = params.cwd_filter.map(str::trim).filter(|s| !s.is_empty());
    let rows = sqlx::query_as!(ClientSessionRow,
        r"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
               conversation_id, resolved_conversation_id, client, cwd, adapter_environment, current_mode, current_task_id,
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
    params.user_id, params.bear_slug, params.include_closed, cwd_filter, limit, params.cursor_updated_at, params.cursor_id)
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
    sqlx::query!(
        r"
        UPDATE client_sessions
        SET adapter_environment = $4,
            updated_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
        user_id,
        bear_id,
        client_session_id,
        adapter_environment
    )
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
    sqlx::query!(
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
        user_id,
        bear_id,
        client_session_id,
        normalized
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_closed(pool: &PgPool, id: Uuid) -> Result<(), DenError> {
    sqlx::query!(
        r"
        UPDATE client_sessions
        SET closed_at = NOW(), updated_at = NOW()
        WHERE id = $1
        ",
        id
    )
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
    let result = sqlx::query!(
        r"
        UPDATE client_sessions
        SET conversation_title = $3,
            conversation_title_updated_at = NOW(),
            conversation_title_synced_at = NULL,
            updated_at = NOW()
        WHERE bear_id = $1
          AND (conversation_id = $2 OR resolved_conversation_id = $2)
        ",
        bear_id,
        conversation_id,
        title
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn list_for_bear_conversation(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
) -> Result<Vec<ClientSessionRow>, DenError> {
    sqlx::query_as!(ClientSessionRow,
        r"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, runtime_session_id,
               conversation_id, resolved_conversation_id, client, cwd, adapter_environment, current_mode, current_task_id,
               conversation_title, conversation_title_updated_at, conversation_title_synced_at,
               closed_at, archived_at, created_at, updated_at
        FROM client_sessions
        WHERE bear_id = $1
          AND (conversation_id = $2 OR resolved_conversation_id = $2)
        ",
    bear_id, conversation_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn mark_title_synced(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
) -> Result<(), DenError> {
    sqlx::query!(
        r"
        UPDATE client_sessions
        SET conversation_title_synced_at = NOW()
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ",
        user_id,
        bear_id,
        client_session_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn resolved_conversation_ids_for_bear(
    pool: &PgPool,
    bear_slug: &str,
) -> Result<Vec<String>, DenError> {
    sqlx::query_scalar!(
        r#"
        SELECT DISTINCT resolved_conversation_id AS "resolved_conversation_id!"
        FROM client_sessions
        WHERE bear_slug = $1
          AND resolved_conversation_id IS NOT NULL
          AND resolved_conversation_id LIKE 'conv-%'
        "#,
        bear_slug
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn mark_archived(pool: &PgPool, id: Uuid) -> Result<(), DenError> {
    sqlx::query!(
        r"
        UPDATE client_sessions
        SET archived_at = NOW(), updated_at = NOW()
        WHERE id = $1
        ",
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod reflection_candidate_tests {
    use super::*;
    use serde_json::json;

    async fn insert_test_user(pool: &PgPool) -> i32 {
        let suffix = Uuid::new_v4().simple().to_string();
        sqlx::query_scalar!(
            r"
            INSERT INTO users (email, username, display_name, passhash)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            ",
            format!("reflection-{suffix}@example.test"),
            format!("reflection{}", &suffix[..16]),
            "Reflection Test User",
            "unused"
        )
        .fetch_one(pool)
        .await
        .expect("insert user")
    }

    async fn insert_test_bear(pool: &PgPool) -> (Uuid, String) {
        let suffix = Uuid::new_v4().simple().to_string();
        let slug = format!("reflection-bear-{}", &suffix[..12]);
        let bear_id = sqlx::query_scalar!(
            r"
            INSERT INTO bears (slug, name, description, system_prompt, live_reflection_enabled)
            VALUES ($1, 'Reflection Test Bear', 'test', 'test', TRUE)
            RETURNING id
            ",
            &slug
        )
        .fetch_one(pool)
        .await
        .expect("insert bear");
        (bear_id, slug)
    }

    async fn insert_open_session(
        pool: &PgPool,
        user_id: i32,
        bear_id: Uuid,
        bear_slug: &str,
        session_id: &str,
        conversation_id: &str,
    ) {
        upsert_session(
            pool,
            UpsertClientSession {
                user_id,
                bear_id,
                bear_slug: bear_slug.to_string(),
                client_session_id: session_id.to_string(),
                runtime_session_id: format!("runtime-{session_id}"),
                conversation_id: conversation_id.to_string(),
                resolved_conversation_id: None,
                client: "test".to_string(),
                cwd: None,
                current_mode: Some(super::ClientSessionMode::Ask),
            },
        )
        .await
        .expect("insert session");
        sqlx::query!("UPDATE client_sessions SET updated_at = NOW() - INTERVAL '1 hour' WHERE client_session_id = $1", session_id)
            .execute(pool)
            .await
            .expect("age session");
    }

    async fn insert_compaction_artifact(
        pool: &PgPool,
        bear_id: Uuid,
        conversation_id: &str,
        end_seq: i64,
    ) {
        let canonical_id = if let Some(id) = sqlx::query_scalar!(
            r"
            SELECT id
            FROM conversations
            WHERE bear_id = $1 AND external_conversation_id = $2
            ",
            bear_id,
            conversation_id
        )
        .fetch_optional(pool)
        .await
        .expect("select conversation")
        {
            id
        } else {
            sqlx::query_scalar!(
                r"
                INSERT INTO conversations (bear_id, external_conversation_id)
                VALUES ($1, $2)
                RETURNING id
                ",
                bear_id,
                conversation_id
            )
            .fetch_one(pool)
            .await
            .expect("insert conversation")
        };
        sqlx::query!(
            r"
            INSERT INTO conversation_compaction_artifacts (
                conversation_id, artifact_kind, policy_version, trigger,
                source_message_start_seq, source_message_end_seq, artifact_json
            )
            VALUES ($1, 'iterative_summary', 'test', 'test', 1, $2, '{}'::jsonb)
            ",
            canonical_id,
            end_seq
        )
        .execute(pool)
        .await
        .expect("insert artifact");
    }

    async fn insert_event(
        pool: &PgPool,
        user_id: i32,
        bear_id: Uuid,
        session_id: &str,
        event_type: &str,
    ) {
        sqlx::query!(
            r"
            INSERT INTO bearwire_events (session_id, bear_id, user_id, event_type, event_json)
            VALUES ($1, $2, $3, $4, '{}'::jsonb)
            ",
            session_id,
            bear_id,
            user_id,
            event_type
        )
        .execute(pool)
        .await
        .expect("insert event");
    }

    async fn insert_reflection_event(
        pool: &PgPool,
        user_id: i32,
        bear_id: Uuid,
        session_id: &str,
        status: &str,
        source_end_seq: Option<i64>,
    ) {
        let mut pair_reflection = json!({ "status": status });
        if let Some(source_end_seq) = source_end_seq {
            pair_reflection["source_message_end_seq"] = json!(source_end_seq);
        }
        sqlx::query!(
            r"
            INSERT INTO bearwire_events (session_id, bear_id, user_id, event_type, event_json)
            VALUES ($1, $2, $3, 'session.reflected', $4)
            ",
            session_id,
            bear_id,
            user_id,
            json!({ "data": { "pair_reflection": pair_reflection } })
        )
        .execute(pool)
        .await
        .expect("insert reflection event");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn activity_threshold_returns_exact_count_only_after_eligibility(pool: PgPool) {
        let user_id = insert_test_user(&pool).await;
        let (bear_id, bear_slug) = insert_test_bear(&pool).await;
        insert_open_session(
            &pool,
            user_id,
            bear_id,
            &bear_slug,
            "session-activity-threshold",
            "conv-activity-threshold",
        )
        .await;

        for _ in 0..3 {
            insert_event(
                &pool,
                user_id,
                bear_id,
                "session-activity-threshold",
                "message.created",
            )
            .await;
        }
        let params = OpenReflectionCandidatesParams {
            stale_after_minutes: 120,
            activity_threshold: 3,
            limit: 25,
        };
        let candidates = list_open_reflection_candidates(&pool, params.clone())
            .await
            .expect("list candidates at threshold");
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.client_session_id == "session-activity-threshold")
            .expect("threshold activity makes session eligible");
        assert_eq!(candidate.event_count, 3);
        assert_eq!(candidate.reflection_trigger, "activity_threshold_sweep");

        insert_event(
            &pool,
            user_id,
            bear_id,
            "session-activity-threshold",
            "message.created",
        )
        .await;
        let candidates = list_open_reflection_candidates(&pool, params)
            .await
            .expect("list candidates above threshold");
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.client_session_id == "session-activity-threshold")
            .expect("session stays eligible above threshold");
        assert_eq!(candidate.event_count, 4);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn skipped_reflection_does_not_hide_later_compaction(pool: PgPool) {
        let user_id = insert_test_user(&pool).await;
        let (bear_id, bear_slug) = insert_test_bear(&pool).await;
        insert_open_session(
            &pool,
            user_id,
            bear_id,
            &bear_slug,
            "session-skipped",
            "conv-skipped",
        )
        .await;
        insert_reflection_event(&pool, user_id, bear_id, "session-skipped", "skipped", None).await;
        insert_compaction_artifact(&pool, bear_id, "conv-skipped", 25).await;

        let candidates = list_open_reflection_candidates(
            &pool,
            OpenReflectionCandidatesParams {
                stale_after_minutes: 30,
                activity_threshold: 20,
                limit: 25,
            },
        )
        .await
        .expect("list candidates");

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.client_session_id == "session-skipped")
            .expect("session remains eligible after later compaction");
        assert_eq!(candidate.latest_compaction_source_end_seq, Some(25));
        assert_eq!(candidate.last_reflected_source_end_seq, None);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn processed_reflection_waits_for_newer_compaction(pool: PgPool) {
        let user_id = insert_test_user(&pool).await;
        let (bear_id, bear_slug) = insert_test_bear(&pool).await;
        insert_open_session(
            &pool,
            user_id,
            bear_id,
            &bear_slug,
            "session-processed",
            "conv-processed",
        )
        .await;
        insert_reflection_event(
            &pool,
            user_id,
            bear_id,
            "session-processed",
            "processed",
            Some(25),
        )
        .await;
        insert_compaction_artifact(&pool, bear_id, "conv-processed", 25).await;

        let candidates = list_open_reflection_candidates(
            &pool,
            OpenReflectionCandidatesParams {
                stale_after_minutes: 30,
                activity_threshold: 20,
                limit: 25,
            },
        )
        .await
        .expect("list candidates");
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.client_session_id != "session-processed"),
            "already-reflected compaction should not be eligible again"
        );

        insert_compaction_artifact(&pool, bear_id, "conv-processed", 30).await;
        let candidates = list_open_reflection_candidates(
            &pool,
            OpenReflectionCandidatesParams {
                stale_after_minutes: 30,
                activity_threshold: 20,
                limit: 25,
            },
        )
        .await
        .expect("list candidates");
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.client_session_id == "session-processed")
            .expect("newer compaction should be eligible");
        assert_eq!(candidate.latest_compaction_source_end_seq, Some(30));
        assert_eq!(candidate.last_reflected_source_end_seq, Some(25));
    }
}

#[cfg(test)]
mod tests;
