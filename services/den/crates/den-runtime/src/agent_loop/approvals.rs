use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone)]
pub struct NativeApprovalRow {
    pub approval_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments_json: Value,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeApprovalDecision {
    Approve,
    Deny,
}

pub async fn create_native_approval(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    acp_session_id: &str,
    tool_call_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> Result<String, DenError> {
    let approval_id = Uuid::new_v4().to_string();
    sqlx::query(
        r"
        INSERT INTO native_runtime_approvals (
            approval_id, bear_id, conversation_id, acp_session_id,
            tool_call_id, tool_name, arguments_json
        ) VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)
        ",
    )
    .bind(&approval_id)
    .bind(bear_id)
    .bind(conversation_id)
    .bind(acp_session_id)
    .bind(tool_call_id)
    .bind(tool_name)
    .bind(arguments.to_string())
    .execute(pool)
    .await
    .map_err(|e| DenError::System(format!("create native approval failed: {e}")))?;
    Ok(approval_id)
}

pub async fn decide_native_approval(
    pool: &PgPool,
    approval_id: &str,
    decision: NativeApprovalDecision,
    reason: Option<&str>,
) -> Result<(), DenError> {
    let status = match decision {
        NativeApprovalDecision::Approve => "approved",
        NativeApprovalDecision::Deny => "denied",
    };
    sqlx::query(
        r"
        UPDATE native_runtime_approvals
        SET status = $2, decision_reason = $3, decided_at = NOW()
        WHERE approval_id = $1
        ",
    )
    .bind(approval_id)
    .bind(status)
    .bind(reason)
    .execute(pool)
    .await
    .map_err(|e| DenError::System(format!("decide native approval failed: {e}")))?;
    Ok(())
}
