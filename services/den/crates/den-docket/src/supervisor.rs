//! Job-run state derivation and idempotent attention records.

use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerivedJobState {
    Running,
    Ready,
    Completed,
    Blocked,
}

pub fn derive_job_state<'a>(
    task_states: impl IntoIterator<Item = &'a str>,
    criteria_complete: bool,
) -> DerivedJobState {
    let states: Vec<_> = task_states.into_iter().collect();
    if states.contains(&"in_progress") {
        return DerivedJobState::Running;
    }
    if states.contains(&"pending") {
        return DerivedJobState::Ready;
    }
    if states.iter().all(|state| *state == "done") && criteria_complete {
        DerivedJobState::Completed
    } else {
        DerivedJobState::Blocked
    }
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct DocketAttention {
    pub id: Uuid,
    pub run_id: Uuid,
    pub task_id: Option<Uuid>,
    pub cause_code: String,
    pub recovery_action: String,
    pub evidence_refs: Value,
    pub resolved_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

/// Unique-open index makes repeated blocked evaluation notification-safe.
pub async fn ensure_attention(
    pool: &PgPool,
    run_id: Uuid,
    task_id: Option<Uuid>,
    cause_code: &str,
    recovery_action: &str,
    evidence_refs: Value,
) -> Result<DocketAttention, DenError> {
    if cause_code.trim().is_empty() || recovery_action.trim().is_empty() {
        return Err(DenError::ValidationError(
            "attention requires cause and recovery action".into(),
        ));
    }
    sqlx::query_as("INSERT INTO docket_attention (run_id, task_id, cause_code, recovery_action, evidence_refs) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (run_id) WHERE resolved_at IS NULL DO UPDATE SET task_id=EXCLUDED.task_id, cause_code=EXCLUDED.cause_code, recovery_action=EXCLUDED.recovery_action, evidence_refs=EXCLUDED.evidence_refs RETURNING id, run_id, task_id, cause_code, recovery_action, evidence_refs, resolved_at, created_at")
        .bind(run_id).bind(task_id).bind(cause_code.trim()).bind(recovery_action.trim()).bind(evidence_refs).fetch_one(pool).await.map_err(Into::into)
}

pub async fn resolve_attention(pool: &PgPool, run_id: Uuid) -> Result<bool, DenError> {
    let result = sqlx::query(
        "UPDATE docket_attention SET resolved_at=now() WHERE run_id=$1 AND resolved_at IS NULL",
    )
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn set_work_run_paused(
    pool: &PgPool,
    work_run_id: Uuid,
    paused: bool,
) -> Result<bool, DenError> {
    let (from, to) = if paused {
        ("running", "paused")
    } else {
        ("paused", "running")
    };
    let result = sqlx::query(
        "UPDATE bear_work_runs SET state=$3, updated_at=now() WHERE id=$1 AND state=$2",
    )
    .bind(work_run_id)
    .bind(from)
    .bind(to)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work_runs::WorkRunState;
    #[test]
    fn empty_queue_is_not_completion() {
        assert_eq!(
            derive_job_state(["blocked"], true),
            DerivedJobState::Blocked
        );
        assert_eq!(
            derive_job_state(["done", "cancelled"], false),
            DerivedJobState::Blocked
        );
        assert_eq!(
            derive_job_state(["done", "done"], true),
            DerivedJobState::Completed
        );
    }

    #[test]
    fn cancelled_required_work_is_not_completion() {
        assert_eq!(
            derive_job_state(["done", "cancelled"], true),
            DerivedJobState::Blocked
        );
    }

    #[test]
    fn paused_runs_remain_active_and_are_not_terminal() {
        assert!(!WorkRunState::Paused.is_terminal());
        assert_eq!(WorkRunState::parse("paused"), Some(WorkRunState::Paused));
    }
}
