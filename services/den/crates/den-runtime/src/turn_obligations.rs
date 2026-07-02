use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnObligationKind {
    ToolResult,
    PermissionDecision,
    HumanInput,
    ResourceBinding,
    HandoffDecision,
}

impl TurnObligationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolResult => "tool_result",
            Self::PermissionDecision => "permission_decision",
            Self::HumanInput => "human_input",
            Self::ResourceBinding => "resource_binding",
            Self::HandoffDecision => "handoff_decision",
        }
    }

    pub fn try_from_storage(value: &str) -> Result<Self, DenError> {
        match value {
            "tool_result" => Ok(Self::ToolResult),
            "permission_decision" => Ok(Self::PermissionDecision),
            "human_input" => Ok(Self::HumanInput),
            "resource_binding" => Ok(Self::ResourceBinding),
            "handoff_decision" => Ok(Self::HandoffDecision),
            other => Err(DenError::ValidationError(format!(
                "unsupported turn obligation kind: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpectedResponderAction {
    ToolResult,
    PermissionDecision,
    HumanInput,
    ResourceBinding,
    HandoffDecision,
}

impl ExpectedResponderAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolResult => "tool_result",
            Self::PermissionDecision => "permission_decision",
            Self::HumanInput => "human_input",
            Self::ResourceBinding => "resource_binding",
            Self::HandoffDecision => "handoff_decision",
        }
    }

    pub fn try_from_storage(value: &str) -> Result<Self, DenError> {
        match value {
            "tool_result" => Ok(Self::ToolResult),
            "permission_decision" => Ok(Self::PermissionDecision),
            "human_input" => Ok(Self::HumanInput),
            "resource_binding" => Ok(Self::ResourceBinding),
            "handoff_decision" => Ok(Self::HandoffDecision),
            other => Err(DenError::ValidationError(format!(
                "unsupported expected responder action: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnObligationState {
    Requested,
    WaitingForClient,
    ResultReceived,
    Continued,
    Failed,
    Cancelled,
}

impl TurnObligationState {
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

    pub fn try_from_storage(value: &str) -> Result<Self, DenError> {
        match value {
            "requested" => Ok(Self::Requested),
            "waiting_for_client" => Ok(Self::WaitingForClient),
            "result_received" => Ok(Self::ResultReceived),
            "continued" => Ok(Self::Continued),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(DenError::ValidationError(format!(
                "unsupported turn obligation state: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnObligationRow {
    pub id: Uuid,
    pub run_id: String,
    pub session_id: String,
    pub kind: String,
    pub expected_responder_action: String,
    pub tool_call_id: Option<String>,
    pub permission_id: Option<String>,
    pub responder_ref_id: Option<String>,
    pub state: String,
    pub turn_step_id: Option<Uuid>,
    pub request_payload: Value,
    pub result_payload: Option<Value>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
}

impl TurnObligationRow {
    pub fn kind_value(&self) -> Result<TurnObligationKind, DenError> {
        TurnObligationKind::try_from_storage(&self.kind)
    }

    pub fn expected_action(&self) -> Result<ExpectedResponderAction, DenError> {
        ExpectedResponderAction::try_from_storage(&self.expected_responder_action)
    }

    pub fn state_value(&self) -> Result<TurnObligationState, DenError> {
        TurnObligationState::try_from_storage(&self.state)
    }
}

fn row_to_obligation(row: sqlx::postgres::PgRow) -> TurnObligationRow {
    TurnObligationRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        session_id: row.get("session_id"),
        kind: row.get("kind"),
        expected_responder_action: row.get("expected_responder_action"),
        tool_call_id: row.get("tool_call_id"),
        permission_id: row.get("permission_id"),
        responder_ref_id: row.try_get("responder_ref_id").ok(),
        state: row.get("state"),
        turn_step_id: row.try_get("turn_step_id").ok(),
        request_payload: row.get("request_payload"),
        result_payload: row.get("result_payload"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
    }
}

pub async fn create_turn_obligation_for_step(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    turn_step_id: Option<Uuid>,
    kind: TurnObligationKind,
    expected_responder_action: ExpectedResponderAction,
    responder_ref_id: &str,
    request_payload: Value,
) -> Result<TurnObligationRow, DenError> {
    let kind = kind.as_str();
    let expected_responder_action = expected_responder_action.as_str();
    let row = sqlx::query(
        r#"
        INSERT INTO turn_obligations (
            run_id, session_id, turn_step_id, kind, expected_responder_action,
            responder_ref_id, state, request_payload
        ) VALUES ($1, $2, $3, $4, $5, $6, 'waiting_for_client', $7)
        RETURNING id, run_id, session_id, kind, expected_responder_action,
                  tool_call_id, permission_id, responder_ref_id, state, turn_step_id,
                  request_payload, result_payload, created_at, updated_at, completed_at
        "#,
    )
    .bind(run_id)
    .bind(session_id)
    .bind(turn_step_id)
    .bind(kind)
    .bind(expected_responder_action)
    .bind(responder_ref_id)
    .bind(request_payload)
    .fetch_one(pool)
    .await?;
    Ok(row_to_obligation(row))
}

pub async fn upsert_tool_result_obligation(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    tool_call_id: &str,
    permission_id: Option<&str>,
    request_payload: Value,
) -> Result<TurnObligationRow, DenError> {
    upsert_tool_result_obligation_for_step(
        pool,
        run_id,
        session_id,
        None,
        tool_call_id,
        permission_id,
        request_payload,
    )
    .await
}

pub async fn upsert_tool_result_obligation_for_step(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    turn_step_id: Option<Uuid>,
    tool_call_id: &str,
    permission_id: Option<&str>,
    request_payload: Value,
) -> Result<TurnObligationRow, DenError> {
    let row = sqlx::query(
        r#"
        INSERT INTO turn_obligations (
            run_id, session_id, turn_step_id, kind, expected_responder_action,
            tool_call_id, permission_id, state, request_payload
        ) VALUES ($1, $2, $3, 'tool_result', 'tool_result', $4, $5, 'waiting_for_client', $6)
        ON CONFLICT (run_id, tool_call_id) WHERE tool_call_id IS NOT NULL
        DO UPDATE SET session_id = EXCLUDED.session_id,
                      turn_step_id = COALESCE(EXCLUDED.turn_step_id, turn_obligations.turn_step_id),
                      expected_responder_action = EXCLUDED.expected_responder_action,
                      permission_id = COALESCE(EXCLUDED.permission_id, turn_obligations.permission_id),
                      state = CASE
                        WHEN turn_obligations.state IN ('result_received','continued','failed','cancelled')
                        THEN turn_obligations.state
                        ELSE EXCLUDED.state
                      END,
                      request_payload = EXCLUDED.request_payload,
                      updated_at = NOW()
        RETURNING id, run_id, session_id, kind, expected_responder_action,
                  tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
                  created_at, updated_at, completed_at
        "#,
    )
    .bind(run_id)
    .bind(session_id)
    .bind(turn_step_id)
    .bind(tool_call_id)
    .bind(permission_id)
    .bind(request_payload)
    .fetch_one(pool)
    .await?;
    Ok(row_to_obligation(row))
}

pub async fn upsert_permission_decision_obligation(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    permission_id: &str,
    tool_call_id: Option<&str>,
    request_payload: Value,
) -> Result<TurnObligationRow, DenError> {
    upsert_permission_decision_obligation_for_step(
        pool,
        run_id,
        session_id,
        None,
        permission_id,
        tool_call_id,
        request_payload,
    )
    .await
}

pub async fn upsert_permission_decision_obligation_for_step(
    pool: &PgPool,
    run_id: &str,
    session_id: &str,
    turn_step_id: Option<Uuid>,
    permission_id: &str,
    tool_call_id: Option<&str>,
    request_payload: Value,
) -> Result<TurnObligationRow, DenError> {
    if let Some(tool_call_id) = tool_call_id {
        if let Some(row) = sqlx::query(
            r#"
            UPDATE turn_obligations
            SET session_id = $2,
                turn_step_id = COALESCE($6, turn_step_id),
                kind = 'permission_decision',
                expected_responder_action = 'permission_decision',
                permission_id = $4,
                state = CASE
                    WHEN state IN ('result_received','continued','failed','cancelled')
                    THEN state
                    ELSE 'waiting_for_client'
                END,
                request_payload = $5,
                updated_at = NOW()
            WHERE run_id = $1
              AND tool_call_id = $3
              AND (permission_id IS NULL OR permission_id = $4)
            RETURNING id, run_id, session_id, kind, expected_responder_action,
                      tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
                      created_at, updated_at, completed_at
            "#,
        )
        .bind(run_id)
        .bind(session_id)
        .bind(tool_call_id)
        .bind(permission_id)
        .bind(request_payload.clone())
        .bind(turn_step_id)
        .fetch_optional(pool)
        .await?
        {
            return Ok(row_to_obligation(row));
        }
    }

    let row = sqlx::query(
        r#"
        INSERT INTO turn_obligations (
            run_id, session_id, turn_step_id, kind, expected_responder_action,
            tool_call_id, permission_id, state, request_payload
        ) VALUES ($1, $2, $3, 'permission_decision', 'permission_decision', $4, $5, 'waiting_for_client', $6)
        ON CONFLICT (run_id, permission_id) WHERE permission_id IS NOT NULL
        DO UPDATE SET session_id = EXCLUDED.session_id,
                      turn_step_id = COALESCE(EXCLUDED.turn_step_id, turn_obligations.turn_step_id),
                      tool_call_id = COALESCE(EXCLUDED.tool_call_id, turn_obligations.tool_call_id),
                      state = CASE
                        WHEN turn_obligations.state IN ('result_received','continued','failed','cancelled')
                        THEN turn_obligations.state
                        ELSE EXCLUDED.state
                      END,
                      request_payload = EXCLUDED.request_payload,
                      updated_at = NOW()
        RETURNING id, run_id, session_id, kind, expected_responder_action,
                  tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
                  created_at, updated_at, completed_at
        "#,
    )
    .bind(run_id)
    .bind(session_id)
    .bind(turn_step_id)
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
) -> Result<Option<TurnObligationRow>, DenError> {
    let row = sqlx::query(
        r#"
        SELECT id, run_id, session_id, kind, expected_responder_action,
               tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
               created_at, updated_at, completed_at
        FROM turn_obligations
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
) -> Result<Option<TurnObligationRow>, DenError> {
    let row = sqlx::query(
        r#"
        SELECT id, run_id, session_id, kind, expected_responder_action,
               tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
               created_at, updated_at, completed_at
        FROM turn_obligations
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
) -> Result<Option<TurnObligationRow>, DenError> {
    let row = sqlx::query(
        r#"
        UPDATE turn_obligations
        SET state = 'result_received',
            result_payload = $2,
            updated_at = NOW()
        WHERE id = $1
          AND state IN ('requested','waiting_for_client','result_received')
        RETURNING id, run_id, session_id, kind, expected_responder_action,
                  tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
                  created_at, updated_at, completed_at
        "#,
    )
    .bind(obligation_id)
    .bind(result_payload)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_obligation))
}

pub async fn mark_waiting_for_tool_result(
    pool: &PgPool,
    obligation_id: Uuid,
) -> Result<Option<TurnObligationRow>, DenError> {
    let row = sqlx::query(
        r#"
        UPDATE turn_obligations
        SET kind = 'tool_result',
            expected_responder_action = 'tool_result',
            state = 'waiting_for_client',
            updated_at = NOW()
        WHERE id = $1
          AND state IN ('requested','waiting_for_client','result_received')
          AND tool_call_id IS NOT NULL
        RETURNING id, run_id, session_id, kind, expected_responder_action,
                  tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
                  created_at, updated_at, completed_at
        "#,
    )
    .bind(obligation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_obligation))
}

pub async fn mark_continued(
    pool: &PgPool,
    obligation_id: Uuid,
) -> Result<Option<TurnObligationRow>, DenError> {
    let row = sqlx::query(
        r#"
        UPDATE turn_obligations
        SET state = 'continued',
            completed_at = COALESCE(completed_at, NOW()),
            updated_at = NOW()
        WHERE id = $1
          AND state IN ('result_received','continued')
        RETURNING id, run_id, session_id, kind, expected_responder_action,
                  tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
                  created_at, updated_at, completed_at
        "#,
    )
    .bind(obligation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_obligation))
}

pub async fn open_client_obligations_for_step(
    pool: &PgPool,
    turn_step_id: Uuid,
) -> Result<Vec<TurnObligationRow>, DenError> {
    let rows = sqlx::query(
        r#"
        SELECT id, run_id, session_id, kind, expected_responder_action,
               tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
               created_at, updated_at, completed_at
        FROM turn_obligations
        WHERE turn_step_id = $1
          AND state IN ('requested','waiting_for_client')
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(turn_step_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_to_obligation).collect())
}

pub async fn open_client_obligations_for_run(
    pool: &PgPool,
    run_id: &str,
) -> Result<Vec<TurnObligationRow>, DenError> {
    let rows = sqlx::query(
        r#"
        SELECT id, run_id, session_id, kind, expected_responder_action,
               tool_call_id, permission_id, state, turn_step_id, request_payload, result_payload,
               created_at, updated_at, completed_at
        FROM turn_obligations
        WHERE run_id = $1
          AND state IN ('requested','waiting_for_client')
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_to_obligation).collect())
}

pub async fn settle_outstanding_for_run(
    pool: &PgPool,
    run_id: &str,
    state: TurnObligationState,
) -> Result<u64, DenError> {
    let state = state.as_str();
    let result = sqlx::query(
        r#"
        UPDATE turn_obligations
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

pub fn obligation_accepts_responder_action(
    obligation: &TurnObligationRow,
    action: ExpectedResponderAction,
) -> bool {
    obligation
        .expected_action()
        .map(|expected| expected == action)
        .unwrap_or(false)
}

pub fn obligation_is_open(obligation: &TurnObligationRow) -> bool {
    matches!(
        obligation.state_value(),
        Ok(TurnObligationState::Requested)
            | Ok(TurnObligationState::WaitingForClient)
            | Ok(TurnObligationState::ResultReceived)
    )
}
