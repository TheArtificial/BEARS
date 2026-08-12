use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::fmt;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanModeState {
    Active,
    Submitted,
    Approved,
    Rejected,
    Cancelled,
}

impl PlanModeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Submitted => "submitted",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "submitted" => Some(Self::Submitted),
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_open(self) -> bool {
        matches!(self, Self::Active | Self::Submitted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClosedPlanModeState {
    Approved,
    Rejected,
    Cancelled,
}

impl ClosedPlanModeState {
    const fn as_state(self) -> PlanModeState {
        match self {
            Self::Approved => PlanModeState::Approved,
            Self::Rejected => PlanModeState::Rejected,
            Self::Cancelled => PlanModeState::Cancelled,
        }
    }

    const fn event_type(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for PlanModeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanModeRequestedBy {
    Pair,
    User,
    System,
}

impl PlanModeRequestedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pair => "pair",
            Self::User => "user",
            Self::System => "system",
        }
    }
}

impl fmt::Display for PlanModeRequestedBy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanModeSessionRow {
    pub id: Uuid,
    pub user_id: i32,
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub client_session_id: String,
    pub state: String,
    pub reason: String,
    pub requested_by: String,
    pub previous_permission_mode: Option<String>,
    pub plan_artifact_path: Option<String>,
    pub plan_title: Option<String>,
    pub plan_body: Option<String>,
    pub approval_request_id: Option<String>,
    pub approved_by_user_id: Option<i32>,
    pub approved_at: Option<OffsetDateTime>,
    pub rejected_at: Option<OffsetDateTime>,
    pub closed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl PlanModeSessionRow {
    pub fn parsed_state(&self) -> Result<PlanModeState, DenError> {
        PlanModeState::parse(&self.state).ok_or_else(|| {
            DenError::System(format!("unknown client plan mode state `{}`", self.state))
        })
    }
}

#[derive(Debug, Clone)]
pub struct EnterPlanModeParams {
    pub user_id: i32,
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub client_session_id: String,
    pub reason: String,
    pub requested_by: PlanModeRequestedBy,
    pub previous_permission_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubmitPlanModeParams {
    pub user_id: i32,
    pub bear_id: Uuid,
    pub client_session_id: String,
    pub plan_mode_id: Option<Uuid>,
    pub title: String,
    pub body: String,
    pub artifact_path: String,
    pub approval_request_id: Option<String>,
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub async fn list_for_bear(
    pool: &PgPool,
    bear_id: Uuid,
    include_closed: bool,
    limit: i64,
) -> Result<Vec<PlanModeSessionRow>, DenError> {
    let limit = limit.clamp(1, 100);
    let rows = sqlx::query_as!(
        PlanModeSessionRow,
        r#"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, state, reason, requested_by,
               previous_permission_mode, plan_artifact_path, plan_title, plan_body,
               approval_request_id, approved_by_user_id, approved_at, rejected_at, closed_at,
               created_at, updated_at
        FROM client_plan_mode_sessions
        WHERE bear_id = $1
          AND ($2 OR state IN ('active', 'submitted'))
        ORDER BY updated_at DESC
        LIMIT $3
        "#,
        bear_id,
        include_closed,
        limit
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn active_for_session(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
) -> Result<Option<PlanModeSessionRow>, DenError> {
    sqlx::query_as!(
        PlanModeSessionRow,
        r#"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, state, reason, requested_by,
               previous_permission_mode, plan_artifact_path, plan_title, plan_body,
               approval_request_id, approved_by_user_id, approved_at, rejected_at, closed_at,
               created_at, updated_at
        FROM client_plan_mode_sessions
        WHERE user_id = $1
          AND bear_id = $2
          AND client_session_id = $3
          AND state IN ('active', 'submitted')
        ORDER BY updated_at DESC
        LIMIT 1
        "#,
        user_id,
        bear_id,
        client_session_id
    )
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn get_by_id_for_bear(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    plan_mode_id: Uuid,
) -> Result<Option<PlanModeSessionRow>, DenError> {
    sqlx::query_as!(
        PlanModeSessionRow,
        r#"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, state, reason, requested_by,
               previous_permission_mode, plan_artifact_path, plan_title, plan_body,
               approval_request_id, approved_by_user_id, approved_at, rejected_at, closed_at,
               created_at, updated_at
        FROM client_plan_mode_sessions
        WHERE id = $1 AND user_id = $2 AND bear_id = $3
        "#,
        plan_mode_id,
        user_id,
        bear_id
    )
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn get_for_session(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    plan_mode_id: Option<Uuid>,
) -> Result<Option<PlanModeSessionRow>, DenError> {
    if let Some(id) = plan_mode_id {
        get_for_session_by_id(pool, user_id, bear_id, client_session_id, id).await
    } else {
        latest_for_session(pool, user_id, bear_id, client_session_id).await
    }
}

async fn get_for_session_by_id(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    plan_mode_id: Uuid,
) -> Result<Option<PlanModeSessionRow>, DenError> {
    sqlx::query_as!(
        PlanModeSessionRow,
        r#"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, state, reason, requested_by,
               previous_permission_mode, plan_artifact_path, plan_title, plan_body,
               approval_request_id, approved_by_user_id, approved_at, rejected_at, closed_at,
               created_at, updated_at
        FROM client_plan_mode_sessions
        WHERE id = $1 AND user_id = $2 AND bear_id = $3 AND client_session_id = $4
        "#,
        plan_mode_id,
        user_id,
        bear_id,
        client_session_id
    )
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn latest_for_session(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
) -> Result<Option<PlanModeSessionRow>, DenError> {
    sqlx::query_as!(
        PlanModeSessionRow,
        r#"
        SELECT id, user_id, bear_id, bear_slug, client_session_id, state, reason, requested_by,
               previous_permission_mode, plan_artifact_path, plan_title, plan_body,
               approval_request_id, approved_by_user_id, approved_at, rejected_at, closed_at,
               created_at, updated_at
        FROM client_plan_mode_sessions
        WHERE user_id = $1 AND bear_id = $2 AND client_session_id = $3
        ORDER BY updated_at DESC
        LIMIT 1
        "#,
        user_id,
        bear_id,
        client_session_id
    )
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn enter_plan_mode(
    pool: &PgPool,
    params: EnterPlanModeParams,
) -> Result<PlanModeSessionRow, DenError> {
    if params.client_session_id.trim().is_empty() {
        return Err(DenError::ValidationError(
            "client_session_id is required".to_string(),
        ));
    }
    if params.bear_slug.trim().is_empty() {
        return Err(DenError::ValidationError(
            "bear_slug is required".to_string(),
        ));
    }

    let mut tx = pool.begin().await?;
    let existing = active_for_session(
        pool,
        params.user_id,
        params.bear_id,
        &params.client_session_id,
    )
    .await?;
    let row = if let Some(existing) = existing {
        existing
    } else {
        sqlx::query_as!(
            PlanModeSessionRow,
            r#"
            INSERT INTO client_plan_mode_sessions (
                user_id, bear_id, bear_slug, client_session_id, state, reason,
                requested_by, previous_permission_mode
            )
            VALUES ($1, $2, $3, $4, 'active', $5, $6, $7)
            RETURNING id, user_id, bear_id, bear_slug, client_session_id, state, reason, requested_by,
                      previous_permission_mode, plan_artifact_path, plan_title, plan_body,
                      approval_request_id, approved_by_user_id, approved_at, rejected_at, closed_at,
                      created_at, updated_at
            "#,
            params.user_id,
            params.bear_id,
            params.bear_slug.trim(),
            params.client_session_id.trim(),
            params.reason.trim(),
            params.requested_by.as_str(),
            clean_optional(params.previous_permission_mode)
        )
        .fetch_one(&mut *tx)
        .await?
    };
    append_event(
        &mut tx,
        &row,
        "entered",
        json!({ "requested_by": params.requested_by.as_str(), "reason": row.reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn submit_plan_artifact(
    pool: &PgPool,
    params: SubmitPlanModeParams,
) -> Result<PlanModeSessionRow, DenError> {
    let title = params.title.trim();
    let body = params.body.trim();
    let artifact_path = params.artifact_path.trim();
    if title.is_empty() {
        return Err(DenError::ValidationError(
            "plan title is required".to_string(),
        ));
    }
    if body.is_empty() {
        return Err(DenError::ValidationError(
            "plan body is required".to_string(),
        ));
    }
    if artifact_path.is_empty()
        || !artifact_path.starts_with("pair/plans/")
        || !artifact_path.ends_with(".md")
    {
        return Err(DenError::ValidationError(
            "plan artifact path must be under pair/plans/ and end with .md".to_string(),
        ));
    }

    let mut tx = pool.begin().await?;
    let current = get_for_session(
        pool,
        params.user_id,
        params.bear_id,
        &params.client_session_id,
        params.plan_mode_id,
    )
    .await?
    .ok_or_else(|| DenError::NotFound("active client plan mode session not found".to_string()))?;
    if !current.parsed_state()?.is_open() {
        return Err(DenError::ValidationError(
            "client plan mode session is already closed".to_string(),
        ));
    }

    let updated = sqlx::query_as!(
        PlanModeSessionRow,
        r#"
        UPDATE client_plan_mode_sessions
        SET state = 'submitted',
            plan_title = $5,
            plan_body = $6,
            plan_artifact_path = $7,
            approval_request_id = $8,
            updated_at = NOW()
        WHERE id = $1 AND user_id = $2 AND bear_id = $3 AND client_session_id = $4
        RETURNING id, user_id, bear_id, bear_slug, client_session_id, state, reason, requested_by,
                  previous_permission_mode, plan_artifact_path, plan_title, plan_body,
                  approval_request_id, approved_by_user_id, approved_at, rejected_at, closed_at,
                  created_at, updated_at
        "#,
        current.id,
        params.user_id,
        params.bear_id,
        params.client_session_id.trim(),
        title,
        body,
        artifact_path,
        clean_optional(params.approval_request_id)
    )
    .fetch_one(&mut *tx)
    .await?;
    append_event(
        &mut tx,
        &updated,
        "artifact_written",
        json!({ "artifact_path": artifact_path, "title": title }),
    )
    .await?;
    append_event(
        &mut tx,
        &updated,
        "exit_requested",
        json!({ "artifact_path": artifact_path, "title": title }),
    )
    .await?;
    tx.commit().await?;
    Ok(updated)
}

pub async fn approve_plan_mode(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    plan_mode_id: Uuid,
) -> Result<PlanModeSessionRow, DenError> {
    let current = current_plan_mode_for_close(
        pool,
        user_id,
        bear_id,
        client_session_id,
        Some(plan_mode_id),
    )
    .await?;
    close_with_state(
        pool,
        user_id,
        bear_id,
        effective_client_session_id(client_session_id, &current),
        plan_mode_id,
        ClosedPlanModeState::Approved,
    )
    .await
}

pub async fn reject_plan_mode(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    plan_mode_id: Uuid,
) -> Result<PlanModeSessionRow, DenError> {
    let current = current_plan_mode_for_close(
        pool,
        user_id,
        bear_id,
        client_session_id,
        Some(plan_mode_id),
    )
    .await?;
    close_with_state(
        pool,
        user_id,
        bear_id,
        effective_client_session_id(client_session_id, &current),
        plan_mode_id,
        ClosedPlanModeState::Rejected,
    )
    .await
}

pub async fn cancel_plan_mode(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    plan_mode_id: Option<Uuid>,
) -> Result<PlanModeSessionRow, DenError> {
    let current =
        current_plan_mode_for_close(pool, user_id, bear_id, client_session_id, plan_mode_id)
            .await?;
    close_with_state(
        pool,
        user_id,
        bear_id,
        &current.client_session_id,
        current.id,
        ClosedPlanModeState::Cancelled,
    )
    .await
}

async fn current_plan_mode_for_close(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    plan_mode_id: Option<Uuid>,
) -> Result<PlanModeSessionRow, DenError> {
    let current = if let Some(plan_mode_id) = plan_mode_id {
        get_by_id_for_bear(pool, user_id, bear_id, plan_mode_id).await?
    } else {
        get_for_session(pool, user_id, bear_id, client_session_id, None).await?
    };
    current.ok_or_else(|| DenError::NotFound("client plan mode session not found".to_string()))
}

fn effective_client_session_id<'a>(requested: &'a str, current: &'a PlanModeSessionRow) -> &'a str {
    if requested.trim().is_empty() {
        &current.client_session_id
    } else {
        requested
    }
}

async fn close_with_state(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    client_session_id: &str,
    plan_mode_id: Uuid,
    state: ClosedPlanModeState,
) -> Result<PlanModeSessionRow, DenError> {
    let event_type = state.event_type();
    let state = state.as_state();
    let mut tx = pool.begin().await?;
    let updated = sqlx::query_as!(
        PlanModeSessionRow,
        r#"
        UPDATE client_plan_mode_sessions
        SET state = $5,
            approved_by_user_id = CASE WHEN $5 = 'approved' THEN $2 ELSE approved_by_user_id END,
            approved_at = CASE WHEN $5 = 'approved' THEN NOW() ELSE approved_at END,
            rejected_at = CASE WHEN $5 = 'rejected' THEN NOW() ELSE rejected_at END,
            closed_at = NOW(),
            updated_at = NOW()
        WHERE id = $1
          AND user_id = $2
          AND bear_id = $3
          AND ($4 = '' OR client_session_id = $4)
          AND state IN ('active', 'submitted')
        RETURNING id, user_id, bear_id, bear_slug, client_session_id, state, reason, requested_by,
                  previous_permission_mode, plan_artifact_path, plan_title, plan_body,
                  approval_request_id, approved_by_user_id, approved_at, rejected_at, closed_at,
                  created_at, updated_at
        "#,
        plan_mode_id,
        user_id,
        bear_id,
        client_session_id.trim(),
        state.as_str()
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DenError::NotFound("open client plan mode session not found".to_string()))?;
    append_event(
        &mut tx,
        &updated,
        event_type,
        json!({ "state": state.as_str() }),
    )
    .await?;
    tx.commit().await?;
    Ok(updated)
}

async fn append_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &PlanModeSessionRow,
    event_type: &str,
    event_payload: Value,
) -> Result<(), DenError> {
    sqlx::query!(
        r#"
        INSERT INTO client_plan_mode_events (
            plan_mode_id, user_id, bear_id, client_session_id, event_type, event_payload
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
        row.id,
        row.user_id,
        row.bear_id,
        &row.client_session_id,
        event_type,
        event_payload
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub fn render_plan_artifact_markdown(title: &str, body: &str) -> String {
    format!("# {}\n\n{}\n", title.trim(), body.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_parse_round_trip() {
        for state in [
            PlanModeState::Active,
            PlanModeState::Submitted,
            PlanModeState::Approved,
            PlanModeState::Rejected,
            PlanModeState::Cancelled,
        ] {
            assert_eq!(PlanModeState::parse(state.as_str()), Some(state));
        }
        assert_eq!(PlanModeState::parse("bogus"), None);
    }

    #[test]
    fn markdown_artifact_has_title_and_body() {
        let rendered = render_plan_artifact_markdown(" My Plan ", " - step one ");
        assert_eq!(rendered, "# My Plan\n\n- step one\n");
    }
}
