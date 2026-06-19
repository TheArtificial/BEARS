use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BearWireObligationState {
    Requested,
    WaitingForClient,
    ResultReceived,
    Continued,
    Failed,
    Cancelled,
}

impl BearWireObligationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::WaitingForClient => "waiting_for_client",
            Self::ResultReceived => "result_received",
            Self::Continued => "continued",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BearWireRunObligationRow {
    pub id: Uuid,
    pub run_id: String,
    pub session_id: String,
    pub kind: String,
    pub expected_client_method: String,
    pub tool_call_id: Option<String>,
    pub permission_id: Option<String>,
    pub state: String,
    pub request_payload: Value,
    pub result_payload: Option<Value>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
}

fn row_to_obligation(row: sqlx::postgres::PgRow) -> BearWireRunObligationRow {
    BearWireRunObligationRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        session_id: row.get("session_id"),
        kind: row.get("kind"),
        expected_client_method: row.get("expected_client_method"),
        tool_call_id: row.get("tool_call_id"),
        permission_id: row.get("permission_id"),
        state: row.get("state"),
        request_payload: row.get("request_payload"),
        result_payload: row.get("result_payload"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
    }
}

pub async fn upsert_tool_call_obligation(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    tool_call_id: &str,
    permission_id: Option<&str>,
    request_payload: Value,
) -> Result<BearWireRunObligationRow, DenError> {
    let row = sqlx::query(
        r#"
        INSERT INTO bearwire_run_obligations (
            run_id, session_id, kind, expected_client_method,
            tool_call_id, permission_id, state, request_payload
        ) VALUES ($1, $2, 'tool_call', 'client.tool.result', $3, $4, 'waiting_for_client', $5)
        ON CONFLICT (run_id, tool_call_id) WHERE tool_call_id IS NOT NULL
        DO UPDATE SET session_id = EXCLUDED.session_id,
                      expected_client_method = EXCLUDED.expected_client_method,
                      permission_id = COALESCE(EXCLUDED.permission_id, bearwire_run_obligations.permission_id),
                      state = CASE
                        WHEN bearwire_run_obligations.state IN ('result_received','continued','failed','cancelled')
                        THEN bearwire_run_obligations.state
                        ELSE EXCLUDED.state
                      END,
                      request_payload = EXCLUDED.request_payload,
                      updated_at = NOW()
        RETURNING id, run_id, session_id, kind, expected_client_method,
                  tool_call_id, permission_id, state, request_payload, result_payload,
                  created_at, updated_at, completed_at
        "#,
    )
    .bind(run_id)
    .bind(session_id)
    .bind(tool_call_id)
    .bind(permission_id)
    .bind(request_payload)
    .fetch_one(pool)
    .await?;
    Ok(row_to_obligation(row))
}

pub async fn upsert_permission_obligation(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    permission_id: &str,
    tool_call_id: Option<&str>,
    request_payload: Value,
) -> Result<BearWireRunObligationRow, DenError> {
    let row = sqlx::query(
        r#"
        INSERT INTO bearwire_run_obligations (
            run_id, session_id, kind, expected_client_method,
            tool_call_id, permission_id, state, request_payload
        ) VALUES ($1, $2, 'permission', 'client.permission.result', $3, $4, 'waiting_for_client', $5)
        ON CONFLICT (run_id, permission_id) WHERE permission_id IS NOT NULL
        DO UPDATE SET session_id = EXCLUDED.session_id,
                      tool_call_id = COALESCE(EXCLUDED.tool_call_id, bearwire_run_obligations.tool_call_id),
                      state = CASE
                        WHEN bearwire_run_obligations.state IN ('result_received','continued','failed','cancelled')
                        THEN bearwire_run_obligations.state
                        ELSE EXCLUDED.state
                      END,
                      request_payload = EXCLUDED.request_payload,
                      updated_at = NOW()
        RETURNING id, run_id, session_id, kind, expected_client_method,
                  tool_call_id, permission_id, state, request_payload, result_payload,
                  created_at, updated_at, completed_at
        "#,
    )
    .bind(run_id)
    .bind(session_id)
    .bind(tool_call_id)
    .bind(permission_id)
    .bind(request_payload)
    .fetch_one(pool)
    .await?;
    Ok(row_to_obligation(row))
}

pub async fn get_tool_call_obligation(
    pool: &PgPool,
    run_id: &str,
    tool_call_id: &str,
) -> Result<Option<BearWireRunObligationRow>, DenError> {
    let row = sqlx::query(
        r#"
        SELECT id, run_id, session_id, kind, expected_client_method,
               tool_call_id, permission_id, state, request_payload, result_payload,
               created_at, updated_at, completed_at
        FROM bearwire_run_obligations
        WHERE run_id = $1 AND tool_call_id = $2
        "#,
    )
    .bind(run_id)
    .bind(tool_call_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_obligation))
}

pub async fn get_permission_obligation(
    pool: &PgPool,
    run_id: &str,
    permission_id: &str,
) -> Result<Option<BearWireRunObligationRow>, DenError> {
    let row = sqlx::query(
        r#"
        SELECT id, run_id, session_id, kind, expected_client_method,
               tool_call_id, permission_id, state, request_payload, result_payload,
               created_at, updated_at, completed_at
        FROM bearwire_run_obligations
        WHERE run_id = $1 AND permission_id = $2
        "#,
    )
    .bind(run_id)
    .bind(permission_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_obligation))
}

pub async fn mark_result_received(
    pool: &PgPool,
    obligation_id: Uuid,
    result_payload: Value,
) -> Result<Option<BearWireRunObligationRow>, DenError> {
    let row = sqlx::query(
        r#"
        UPDATE bearwire_run_obligations
        SET state = 'result_received',
            result_payload = $2,
            updated_at = NOW()
        WHERE id = $1
          AND state IN ('requested','waiting_for_client','result_received')
        RETURNING id, run_id, session_id, kind, expected_client_method,
                  tool_call_id, permission_id, state, request_payload, result_payload,
                  created_at, updated_at, completed_at
        "#,
    )
    .bind(obligation_id)
    .bind(result_payload)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_obligation))
}

pub async fn mark_continued(
    pool: &PgPool,
    obligation_id: Uuid,
) -> Result<Option<BearWireRunObligationRow>, DenError> {
    let row = sqlx::query(
        r#"
        UPDATE bearwire_run_obligations
        SET state = 'continued',
            completed_at = COALESCE(completed_at, NOW()),
            updated_at = NOW()
        WHERE id = $1
          AND state IN ('result_received','continued')
        RETURNING id, run_id, session_id, kind, expected_client_method,
                  tool_call_id, permission_id, state, request_payload, result_payload,
                  created_at, updated_at, completed_at
        "#,
    )
    .bind(obligation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_obligation))
}

pub async fn settle_outstanding_for_run(
    pool: &PgPool,
    run_id: &str,
    state: BearWireObligationState,
) -> Result<u64, DenError> {
    let state = state.as_str();
    let result = sqlx::query(
        r#"
        UPDATE bearwire_run_obligations
        SET state = $2,
            completed_at = COALESCE(completed_at, NOW()),
            updated_at = NOW()
        WHERE run_id = $1
          AND state IN ('requested','waiting_for_client','result_received')
        "#,
    )
    .bind(run_id)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub fn obligation_accepts_client_method(
    obligation: &BearWireRunObligationRow,
    method: &str,
) -> bool {
    obligation.expected_client_method == method
}

pub fn obligation_is_open(obligation: &BearWireRunObligationRow) -> bool {
    matches!(
        obligation.state.as_str(),
        "requested" | "waiting_for_client" | "result_received"
    )
}
