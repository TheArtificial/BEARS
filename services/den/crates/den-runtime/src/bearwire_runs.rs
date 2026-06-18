use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BearWireRunState {
    Accepted,
    Running,
    WaitingForToolResult,
    WaitingForPermission,
    Continuing,
    Completed,
    Failed,
    Cancelled,
}

impl BearWireRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::WaitingForToolResult => "waiting_for_tool_result",
            Self::WaitingForPermission => "waiting_for_permission",
            Self::Continuing => "continuing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BearWireRunRow {
    pub id: Uuid,
    pub run_id: String,
    pub session_id: String,
    pub bear_id: Uuid,
    pub user_id: i32,
    pub state: String,
    pub active_tool_call_id: Option<String>,
    pub active_permission_id: Option<String>,
    pub active_request_id: Option<Uuid>,
    pub terminal_reason: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
}

fn row_to_run(row: sqlx::postgres::PgRow) -> BearWireRunRow {
    BearWireRunRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        session_id: row.get("session_id"),
        bear_id: row.get("bear_id"),
        user_id: row.get("user_id"),
        state: row.get("state"),
        active_tool_call_id: row.get("active_tool_call_id"),
        active_permission_id: row.get("active_permission_id"),
        active_request_id: row.get("active_request_id"),
        terminal_reason: row.get("terminal_reason"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
    }
}

pub async fn create_run(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    bear_id: Uuid,
    user_id: i32,
) -> Result<BearWireRunRow, DenError> {
    let row = sqlx::query(
        r#"
        INSERT INTO bearwire_runs (run_id, session_id, bear_id, user_id, state)
        VALUES ($1, $2, $3, $4, 'accepted')
        RETURNING id, run_id, session_id, bear_id, user_id, state,
                  active_tool_call_id, active_permission_id, active_request_id,
                  terminal_reason, created_at, updated_at, completed_at
        "#,
    )
    .bind(run_id)
    .bind(session_id)
    .bind(bear_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(row_to_run(row))
}

pub async fn get_run(pool: &PgPool, run_id: &str) -> Result<Option<BearWireRunRow>, DenError> {
    let row = sqlx::query(
        r#"
        SELECT id, run_id, session_id, bear_id, user_id, state,
               active_tool_call_id, active_permission_id, active_request_id,
               terminal_reason, created_at, updated_at, completed_at
        FROM bearwire_runs
        WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}

pub async fn active_run_for_session(
    pool: &PgPool,
    session_id: &str,
) -> Result<Option<BearWireRunRow>, DenError> {
    let row = sqlx::query(
        r#"
        SELECT id, run_id, session_id, bear_id, user_id, state,
               active_tool_call_id, active_permission_id, active_request_id,
               terminal_reason, created_at, updated_at, completed_at
        FROM bearwire_runs
        WHERE session_id = $1
          AND state IN ('accepted','running','waiting_for_tool_result','waiting_for_permission','continuing')
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}

pub async fn transition_run(
    pool: &PgPool,
    run_id: &str,
    state: BearWireRunState,
    active_tool_call_id: Option<&str>,
    active_permission_id: Option<&str>,
    active_request_id: Option<Uuid>,
    terminal_reason: Option<&str>,
) -> Result<Option<BearWireRunRow>, DenError> {
    let terminal = matches!(
        state,
        BearWireRunState::Completed | BearWireRunState::Failed | BearWireRunState::Cancelled
    );
    let row = sqlx::query(
        r#"
        UPDATE bearwire_runs
        SET state = $2,
            active_tool_call_id = $3,
            active_permission_id = $4,
            active_request_id = $5,
            terminal_reason = $6,
            updated_at = NOW(),
            completed_at = CASE WHEN $7 THEN COALESCE(completed_at, NOW()) ELSE completed_at END
        WHERE run_id = $1
        RETURNING id, run_id, session_id, bear_id, user_id, state,
                  active_tool_call_id, active_permission_id, active_request_id,
                  terminal_reason, created_at, updated_at, completed_at
        "#,
    )
    .bind(run_id)
    .bind(state.as_str())
    .bind(active_tool_call_id)
    .bind(active_permission_id)
    .bind(active_request_id)
    .bind(terminal_reason)
    .bind(terminal)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}
