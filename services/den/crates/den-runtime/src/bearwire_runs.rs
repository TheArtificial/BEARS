use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
        terminal_reason: row.get("terminal_reason"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
    }
}

const RUN_RETURNING: &str = r#"
    id, run_id, session_id, bear_id, user_id, state,
    terminal_reason, created_at, updated_at, completed_at
"#;

pub async fn create_run(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    bear_id: Uuid,
    user_id: i32,
) -> Result<BearWireRunRow, DenError> {
    let row = sqlx::query(&format!(
        r#"
        INSERT INTO bearwire_runs (run_id, session_id, bear_id, user_id, state)
        VALUES ($1, $2, $3, $4, 'accepted')
        RETURNING {RUN_RETURNING}
        "#
    ))
    .bind(run_id)
    .bind(session_id)
    .bind(bear_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(row_to_run(row))
}

pub async fn get_run(pool: &PgPool, run_id: &str) -> Result<Option<BearWireRunRow>, DenError> {
    let row = sqlx::query(&format!(
        r#"
        SELECT {RUN_RETURNING}
        FROM bearwire_runs
        WHERE run_id = $1
        "#
    ))
    .bind(run_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}

pub async fn active_run_for_session(
    pool: &PgPool,
    session_id: &str,
) -> Result<Option<BearWireRunRow>, DenError> {
    let row = sqlx::query(&format!(
        r#"
        SELECT {RUN_RETURNING}
        FROM bearwire_runs
        WHERE session_id = $1
          AND state IN ('accepted','running','waiting_for_tool_result','waiting_for_permission','continuing')
        ORDER BY created_at DESC
        LIMIT 1
        "#
    ))
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}

pub async fn supersede_active_run_for_session(
    pool: &PgPool,
    session_id: &str,
    bear_id: Uuid,
    user_id: i32,
    reason: &str,
) -> Result<Option<BearWireRunRow>, DenError> {
    let row = sqlx::query(&format!(
        r#"
        UPDATE bearwire_runs
        SET state = 'failed', terminal_reason = $4, completed_at = NOW(), updated_at = NOW()
        WHERE id = (
            SELECT id
            FROM bearwire_runs
            WHERE session_id = $1 AND bear_id = $2 AND user_id = $3
              AND state IN ('accepted','running','waiting_for_tool_result','waiting_for_permission','continuing')
            ORDER BY created_at DESC
            LIMIT 1
        )
        RETURNING {RUN_RETURNING}
        "#
    ))
    .bind(session_id)
    .bind(bear_id)
    .bind(user_id)
    .bind(reason)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}

#[derive(Debug, Clone, Serialize)]
pub struct BearWireClientResultRow {
    pub id: Uuid,
    pub run_id: String,
    pub obligation_kind: String,
    pub obligation_id: String,
    pub result_hash: String,
    pub payload_json: serde_json::Value,
    pub turn_step_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BearWireClientResultRecord {
    Inserted { row: BearWireClientResultRow },
    DuplicateIdentical { row: BearWireClientResultRow },
    DuplicateConflict { existing_hash: String },
}

fn result_hash(payload: &serde_json::Value) -> Result<String, DenError> {
    let bytes = serde_json::to_vec(payload).map_err(|err| {
        DenError::System(format!("serialize BearWire client result failed: {err}"))
    })?;
    let digest = Sha256::digest(&bytes);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
}

fn row_to_client_result(row: sqlx::postgres::PgRow) -> BearWireClientResultRow {
    BearWireClientResultRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        obligation_kind: row.get("obligation_kind"),
        obligation_id: row.get("obligation_id"),
        result_hash: row.get("result_hash"),
        payload_json: row.get("payload_json"),
        turn_step_id: row.try_get("turn_step_id").ok(),
        created_at: row.get("created_at"),
    }
}

pub async fn existing_client_result_for_payload(
    pool: &PgPool,
    run_id: &str,
    obligation_kind: &str,
    obligation_id: &str,
    payload_json: &serde_json::Value,
) -> Result<Option<BearWireClientResultRecord>, DenError> {
    let Some(existing) = sqlx::query(
        r#"
        SELECT id, run_id, obligation_kind, obligation_id, result_hash, payload_json, turn_step_id, created_at
        FROM bearwire_client_results
        WHERE run_id = $1 AND obligation_kind = $2 AND obligation_id = $3
        "#,
    )
    .bind(run_id)
    .bind(obligation_kind)
    .bind(obligation_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let row = row_to_client_result(existing);
    if row.result_hash == result_hash(payload_json)? {
        Ok(Some(BearWireClientResultRecord::DuplicateIdentical { row }))
    } else {
        Ok(Some(BearWireClientResultRecord::DuplicateConflict {
            existing_hash: row.result_hash,
        }))
    }
}

pub async fn record_client_result(
    pool: &PgPool,
    run_id: &str,
    obligation_kind: &str,
    obligation_id: &str,
    payload_json: serde_json::Value,
) -> Result<BearWireClientResultRecord, DenError> {
    record_client_result_for_step(
        pool,
        run_id,
        None,
        obligation_kind,
        obligation_id,
        payload_json,
    )
    .await
}

pub async fn record_client_result_for_step(
    pool: &PgPool,
    run_id: &str,
    turn_step_id: Option<Uuid>,
    obligation_kind: &str,
    obligation_id: &str,
    payload_json: serde_json::Value,
) -> Result<BearWireClientResultRecord, DenError> {
    let hash = result_hash(&payload_json)?;
    let inserted = sqlx::query(
        r#"
        INSERT INTO bearwire_client_results (
            run_id, turn_step_id, obligation_kind, obligation_id, result_hash, payload_json
        ) VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (run_id, obligation_kind, obligation_id) DO NOTHING
        RETURNING id, run_id, obligation_kind, obligation_id, result_hash, payload_json, turn_step_id, created_at
        "#,
    )
    .bind(run_id)
    .bind(turn_step_id)
    .bind(obligation_kind)
    .bind(obligation_id)
    .bind(&hash)
    .bind(&payload_json)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = inserted {
        return Ok(BearWireClientResultRecord::Inserted {
            row: row_to_client_result(row),
        });
    }

    let existing = sqlx::query(
        r#"
        SELECT id, run_id, obligation_kind, obligation_id, result_hash, payload_json, turn_step_id, created_at
        FROM bearwire_client_results
        WHERE run_id = $1 AND obligation_kind = $2 AND obligation_id = $3
        "#,
    )
    .bind(run_id)
    .bind(obligation_kind)
    .bind(obligation_id)
    .fetch_one(pool)
    .await?;
    let row = row_to_client_result(existing);
    if row.result_hash == hash {
        Ok(BearWireClientResultRecord::DuplicateIdentical { row })
    } else {
        Ok(BearWireClientResultRecord::DuplicateConflict {
            existing_hash: row.result_hash,
        })
    }
}

pub async fn client_result_count_for_run_kind(
    pool: &PgPool,
    run_id: &str,
    obligation_kind: &str,
) -> Result<i64, DenError> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM bearwire_client_results
        WHERE run_id = $1 AND obligation_kind = $2
        "#,
    )
    .bind(run_id)
    .bind(obligation_kind)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn transition_run(
    pool: &PgPool,
    run_id: &str,
    state: BearWireRunState,
    terminal_reason: Option<&str>,
) -> Result<Option<BearWireRunRow>, DenError> {
    let terminal = matches!(
        state,
        BearWireRunState::Completed | BearWireRunState::Failed | BearWireRunState::Cancelled
    );
    let row = sqlx::query(&format!(
        r#"
        UPDATE bearwire_runs
        SET state = $2,
            terminal_reason = $3,
            updated_at = NOW(),
            completed_at = CASE WHEN $4 THEN COALESCE(completed_at, NOW()) ELSE completed_at END
        WHERE run_id = $1
        RETURNING {RUN_RETURNING}
        "#
    ))
    .bind(run_id)
    .bind(state.as_str())
    .bind(terminal_reason)
    .bind(terminal)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_run))
}
