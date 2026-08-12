//! Durable attempt and result-rollup primitives for ADR-0056 recovery.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

use crate::execution_profiles::{ExecutionProfile, ResolvedExecutionProfile};

pub const MAX_TURN_ATTEMPTS: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscalationDecision {
    Complete,
    Retry { profile: Option<ExecutionProfile> },
    Handoff,
}

/// Persisted supervisor action. Model output is evidence only; this value is
/// selected from a settled attempt by deterministic Den policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorDisposition {
    Complete,
    Retry,
    Escalate,
    Handoff,
    AwaitRecovery,
}

impl SupervisorDisposition {
    fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Retry => "retry",
            Self::Escalate => "escalate",
            Self::Handoff => "handoff",
            Self::AwaitRecovery => "await_recovery",
        }
    }

    fn retry_disposition(self) -> RetryDisposition {
        match self {
            Self::Complete => RetryDisposition::None,
            Self::Retry => RetryDisposition::Retry,
            Self::Escalate => RetryDisposition::Escalate,
            Self::Handoff => RetryDisposition::Handoff,
            Self::AwaitRecovery => RetryDisposition::Pause,
        }
    }
}

pub fn disposition_for(decision: EscalationDecision) -> SupervisorDisposition {
    match decision {
        EscalationDecision::Complete => SupervisorDisposition::Complete,
        EscalationDecision::Retry { profile: None } => SupervisorDisposition::Retry,
        EscalationDecision::Retry { profile: Some(_) } => SupervisorDisposition::Escalate,
        EscalationDecision::Handoff => SupervisorDisposition::Handoff,
    }
}

/// Supervisor-owned retry policy. Only normalized infrastructure/capability
/// failures may increase the symbolic profile, and each attempt advances at
/// most one tier. Unknown-profile fallback retries without inventing a model.
pub fn decide_escalation(
    profile: Option<ExecutionProfile>,
    attempt: i32,
    outcome: AttemptOutcome,
    cause_code: &str,
) -> EscalationDecision {
    if outcome == AttemptOutcome::Completed {
        return EscalationDecision::Complete;
    }
    if attempt >= MAX_TURN_ATTEMPTS || !escalation_eligible(outcome, cause_code) {
        return EscalationDecision::Handoff;
    }
    match profile {
        Some(current) => current
            .next()
            .map(|profile| EscalationDecision::Retry {
                profile: Some(profile),
            })
            .unwrap_or(EscalationDecision::Handoff),
        None => EscalationDecision::Retry { profile: None },
    }
}

fn escalation_eligible(outcome: AttemptOutcome, cause_code: &str) -> bool {
    matches!(outcome, AttemptOutcome::Failed | AttemptOutcome::TimedOut)
        && matches!(
            cause_code,
            "activity_timeout" | "context_exhausted" | "model_unavailable" | "provider_error"
        )
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct TurnAttempt {
    pub id: Uuid,
    pub routing_decision_id: Uuid,
    pub work_run_id: Option<Uuid>,
    pub attempt: i32,
    pub state: String,
    pub outcome: Option<String>,
    pub cause_code: Option<String>,
    pub retry_disposition: Option<String>,
    pub evidence_refs: Option<Value>,
    pub resolved_profile: Option<String>,
    pub profile_provenance: String,
    pub latency_ms: Option<i64>,
    pub cost_microusd: Option<i64>,
    pub started_at: OffsetDateTime,
    pub last_activity_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
}

struct AttentionOutboxRow {
    run_id: Uuid,
    task_id: Uuid,
    cause_code: String,
    evidence_refs: Value,
    supervisor_disposition: String,
}

const CLAIM_LEASE_SECONDS: i64 = 15 * 60;

/// Atomically reserve the invocation authority and create its durable attempt
/// before a worker can perform model or tool side effects.
pub async fn claim_turn_attempt(
    pool: &PgPool,
    routing_decision_id: Uuid,
    work_run_id: Option<Uuid>,
    attempt: i32,
    resolved: ResolvedExecutionProfile,
) -> Result<TurnAttempt, DenError> {
    if attempt < 1 {
        return Err(DenError::ValidationError(
            "turn attempt must be positive".into(),
        ));
    }
    let owner_id = format!("work-run:{}", work_run_id.unwrap_or(routing_decision_id));
    let profile = resolved.profile.map(ExecutionProfile::as_str);
    let provenance = resolved.provenance.as_str();
    let mut tx = pool.begin().await?;
    let decision = sqlx::query!(
        "SELECT bear_id AS \"bear_id!: Uuid\", job_id AS \"job_id!: Uuid\", run_id AS \"run_id!: Uuid\", task_id AS \"task_id!: Uuid\" FROM docket_routing_decisions WHERE id = $1 FOR UPDATE",
        routing_decision_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DenError::NotFound(format!("routing decision {routing_decision_id}")))?;
    let claim_id: Uuid = sqlx::query_scalar!(
        "INSERT INTO docket_turn_claims (routing_decision_id, bear_id, job_id, run_id, task_id, work_run_id, owner_id, lease_expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7, now() + make_interval(secs => $8))
         ON CONFLICT (routing_decision_id) DO UPDATE
         SET lease_expires_at = EXCLUDED.lease_expires_at, updated_at = now()
         WHERE docket_turn_claims.owner_id = EXCLUDED.owner_id
           AND docket_turn_claims.state IN ('reserved', 'executing')
         RETURNING id",
        routing_decision_id, decision.bear_id, decision.job_id, decision.run_id, decision.task_id,
        work_run_id, owner_id, CLAIM_LEASE_SECONDS as f64
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DenError::ValidationError("turn is already claimed by another worker".into()))?;
    let row = sqlx::query_as!(
        TurnAttempt,
        "INSERT INTO docket_turn_attempts (routing_decision_id, work_run_id, attempt, claim_id, state, resolved_profile, profile_provenance)
         VALUES ($1,$2,$3,$4,'executing',$5,$6)
         ON CONFLICT (routing_decision_id, attempt) DO UPDATE
         SET last_activity_at = now()
         WHERE docket_turn_attempts.claim_id = EXCLUDED.claim_id
         RETURNING id, routing_decision_id, work_run_id, attempt, state, outcome, cause_code, retry_disposition,
                   evidence_refs AS \"evidence_refs: _\", resolved_profile, profile_provenance, latency_ms,
                   cost_microusd, started_at, last_activity_at, finished_at",
        routing_decision_id, work_run_id, attempt, claim_id, profile, provenance
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DenError::ValidationError("turn attempt belongs to another claim".into()))?;
    sqlx::query!(
        "UPDATE docket_turn_claims SET state = 'executing', updated_at = now() WHERE id = $1",
        claim_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn record_turn_activity(
    pool: &PgPool,
    attempt_id: Uuid,
    evidence: Value,
) -> Result<bool, DenError> {
    let result = sqlx::query!(
        "UPDATE docket_turn_attempts
         SET last_activity_at = now(), evidence_refs = COALESCE(evidence_refs, '{}'::jsonb) || $2::jsonb
         WHERE id = $1 AND state = 'executing'
           AND EXISTS (
               SELECT 1 FROM docket_turn_claims
               WHERE id = docket_turn_attempts.claim_id
                 AND state = 'executing' AND lease_expires_at > now()
           )",
        attempt_id,
        evidence
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Completed,
    Blocked,
    Failed,
    TimedOut,
    Cancelled,
}
impl AttemptOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "completed" => Some(Self::Completed),
            "blocked" => Some(Self::Blocked),
            "failed" => Some(Self::Failed),
            "timed_out" => Some(Self::TimedOut),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryDisposition {
    None,
    Retry,
    Escalate,
    Handoff,
    Pause,
}
impl RetryDisposition {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Retry => "retry",
            Self::Escalate => "escalate",
            Self::Handoff => "handoff",
            Self::Pause => "pause",
        }
    }
}

/// Compare-and-set terminalization. False means another worker already won.
pub async fn terminalize_turn_attempt(
    pool: &PgPool,
    attempt_id: Uuid,
    outcome: AttemptOutcome,
    cause_code: &str,
    disposition: RetryDisposition,
    evidence: Value,
    latency_ms: Option<i64>,
    cost_microusd: Option<i64>,
) -> Result<bool, DenError> {
    if cause_code.trim().is_empty() {
        return Err(DenError::ValidationError(
            "terminal attempt requires a cause code".into(),
        ));
    }
    if latency_ms.is_some_and(|value| value < 0) || cost_microusd.is_some_and(|value| value < 0) {
        return Err(DenError::ValidationError(
            "attempt attribution cannot be negative".into(),
        ));
    }
    let result = sqlx::query!(
        "WITH settled AS (
             UPDATE docket_turn_attempts
             SET state='settled', outcome=$2, cause_code=$3, retry_disposition=$4,
                 evidence_refs=COALESCE(evidence_refs, '{}'::jsonb) || $5::jsonb,
                 latency_ms=$6, cost_microusd=$7, finished_at=now(), last_activity_at=now()
             WHERE id=$1 AND state='executing'
               AND EXISTS (
                   SELECT 1 FROM docket_turn_claims
                   WHERE id = docket_turn_attempts.claim_id
                     AND state = 'executing' AND lease_expires_at > now()
               )
             RETURNING claim_id
         )
         UPDATE docket_turn_claims
         SET state='settled', updated_at=now()
         WHERE id IN (SELECT claim_id FROM settled WHERE claim_id IS NOT NULL)",
        attempt_id,
        outcome.as_str(),
        cause_code,
        disposition.as_str(),
        evidence,
        latency_ms,
        cost_microusd
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Decide the next supervisor action from a durable terminal attempt. This is
/// read-only and deterministic, so crash recovery cannot ask the model to
/// choose whether it should retry or escalate.
pub async fn escalation_for_attempt(
    pool: &PgPool,
    attempt_id: Uuid,
) -> Result<EscalationDecision, DenError> {
    let row = sqlx::query!(
        "SELECT attempt AS \"attempt!: i32\", resolved_profile, outcome, cause_code AS \"cause_code!: String\"
         FROM docket_turn_attempts
         WHERE id = $1 AND state = 'settled'",
        attempt_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DenError::NotFound(format!("terminal turn attempt {attempt_id}")))?;
    let outcome = row
        .outcome
        .as_deref()
        .and_then(AttemptOutcome::parse)
        .ok_or_else(|| DenError::ValidationError("terminal attempt has invalid outcome".into()))?;
    Ok(decide_escalation(
        row.resolved_profile
            .as_deref()
            .and_then(ExecutionProfile::parse),
        row.attempt,
        outcome,
        &row.cause_code,
    ))
}

pub async fn apply_supervisor_disposition(
    pool: &PgPool,
    attempt_id: Uuid,
) -> Result<SupervisorDisposition, DenError> {
    let decision = escalation_for_attempt(pool, attempt_id).await?;
    let disposition = disposition_for(decision);
    let retry_disposition = disposition.retry_disposition();
    let result = sqlx::query!(
        "UPDATE docket_turn_attempts
         SET supervisor_disposition = $2, retry_disposition = $3
         WHERE id = $1 AND state = 'settled'
           AND (supervisor_disposition IS NULL OR supervisor_disposition = $2)",
        attempt_id,
        disposition.as_str(),
        retry_disposition.as_str()
    )
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        let existing: Option<String> = sqlx::query_scalar!(
            "SELECT supervisor_disposition FROM docket_turn_attempts WHERE id = $1",
            attempt_id
        )
        .fetch_optional(pool)
        .await?
        .flatten();
        return match existing.as_deref() {
            Some(value) if value == disposition.as_str() => Ok(disposition),
            Some(_) => Err(DenError::ValidationError(
                "settled attempt already has a conflicting supervisor disposition".into(),
            )),
            None => Err(DenError::NotFound(format!(
                "settled turn attempt {attempt_id}"
            ))),
        };
    }
    Ok(disposition)
}

/// Atomically persists the task-correlated attention and channel-independent
/// delivery intent. Replaying the same terminal attempt is a no-op.
pub async fn persist_attention_outbox(
    pool: &PgPool,
    attempt_id: Uuid,
    recovery_action: &str,
) -> Result<bool, DenError> {
    if recovery_action.trim().is_empty() {
        return Err(DenError::ValidationError(
            "recovery action is required".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as!(
        AttentionOutboxRow,
        "SELECT d.run_id AS \"run_id!: Uuid\", d.task_id AS \"task_id!: Uuid\", a.cause_code AS \"cause_code!: String\",
                COALESCE(a.evidence_refs, '{}'::jsonb) AS \"evidence_refs!: _\",
                a.supervisor_disposition AS \"supervisor_disposition!: String\"
         FROM docket_turn_attempts a
         JOIN docket_routing_decisions d ON d.id = a.routing_decision_id
         WHERE a.id = $1 AND a.state = 'settled' AND a.supervisor_disposition IS NOT NULL
         FOR UPDATE",
        attempt_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DenError::ValidationError("attempt must be settled and supervisor-disposed".into()))?;
    sqlx::query!(
        "INSERT INTO docket_attention (run_id, task_id, cause_code, recovery_action, evidence_refs)
         VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (run_id) WHERE resolved_at IS NULL DO UPDATE
         SET task_id = EXCLUDED.task_id, cause_code = EXCLUDED.cause_code,
             recovery_action = EXCLUDED.recovery_action, evidence_refs = EXCLUDED.evidence_refs",
        row.run_id,
        row.task_id,
        row.cause_code,
        recovery_action.trim(),
        row.evidence_refs
    )
    .execute(&mut *tx)
    .await?;
    let deduplication_key = format!("turn-attempt:{attempt_id}:attention");
    let result = sqlx::query!(
        "INSERT INTO docket_notification_outbox (run_id, task_id, deduplication_key, kind, payload)
         VALUES ($1,$2,$3,'attention',$4)
         ON CONFLICT (deduplication_key) DO NOTHING",
        row.run_id,
        row.task_id,
        deduplication_key,
        json!({"attempt_id": attempt_id, "cause_code": row.cause_code, "disposition": row.supervisor_disposition, "recovery_action": recovery_action.trim()})
    )
    .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

pub async fn terminalize_stale_attempts(
    pool: &PgPool,
    stale_before: OffsetDateTime,
) -> Result<u64, DenError> {
    let result = sqlx::query!(
        "WITH settled AS (
             UPDATE docket_turn_attempts
             SET state='settled', outcome='timed_out', cause_code='activity_timeout',
                 retry_disposition='pause', supervisor_disposition='await_recovery',
                 evidence_refs=COALESCE(evidence_refs, '{}'::jsonb) || $2::jsonb,
                 finished_at=now(), last_activity_at=now()
             WHERE state='executing' AND last_activity_at < $1
             RETURNING claim_id
         )
         UPDATE docket_turn_claims
         SET state='settled', updated_at=now()
         WHERE id IN (SELECT claim_id FROM settled WHERE claim_id IS NOT NULL)",
        stale_before,
        json!({"synthetic": true, "boundary": "watchdog"})
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultRollup {
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Value,
}

pub async fn persist_result_rollup(
    pool: &PgPool,
    run_id: Uuid,
    task_id: Uuid,
    parent_task_id: Uuid,
    rollup: ResultRollup,
) -> Result<bool, DenError> {
    if rollup.summary.trim().is_empty() {
        return Err(DenError::ValidationError(
            "result rollup requires a summary".into(),
        ));
    }
    let result = sqlx::query!(
        "INSERT INTO docket_result_rollups (run_id, task_id, parent_task_id, summary, evidence_refs) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (run_id, task_id) DO NOTHING",
        run_id,
        task_id,
        parent_task_id,
        rollup.summary.trim(),
        rollup.evidence_refs
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn parent_rollup_context(
    pool: &PgPool,
    run_id: Uuid,
    parent_task_id: Uuid,
) -> Result<Vec<ResultRollup>, DenError> {
    let rows = sqlx::query_as!(
        ResultRollup,
        "SELECT summary AS \"summary!: String\", evidence_refs AS \"evidence_refs!: _\" FROM docket_result_rollups WHERE run_id=$1 AND parent_task_id=$2 ORDER BY created_at, task_id",
        run_id,
        parent_task_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_escalates_only_eligible_failures_one_tier() {
        assert_eq!(
            decide_escalation(
                Some(ExecutionProfile::Economy),
                1,
                AttemptOutcome::Failed,
                "provider_error",
            ),
            EscalationDecision::Retry {
                profile: Some(ExecutionProfile::Balanced)
            }
        );
        assert_eq!(
            decide_escalation(
                Some(ExecutionProfile::Balanced),
                2,
                AttemptOutcome::TimedOut,
                "activity_timeout",
            ),
            EscalationDecision::Retry {
                profile: Some(ExecutionProfile::Advanced)
            }
        );
    }

    #[test]
    fn supervisor_stops_at_ceiling_or_noneligible_outcome() {
        assert_eq!(
            decide_escalation(
                Some(ExecutionProfile::Advanced),
                1,
                AttemptOutcome::Failed,
                "provider_error",
            ),
            EscalationDecision::Handoff
        );
        assert_eq!(
            decide_escalation(
                Some(ExecutionProfile::Economy),
                MAX_TURN_ATTEMPTS,
                AttemptOutcome::Failed,
                "provider_error",
            ),
            EscalationDecision::Handoff
        );
        assert_eq!(
            decide_escalation(
                Some(ExecutionProfile::Economy),
                1,
                AttemptOutcome::Blocked,
                "approval_required",
            ),
            EscalationDecision::Handoff
        );
    }

    #[test]
    fn successful_attempt_never_retries() {
        assert_eq!(
            decide_escalation(
                Some(ExecutionProfile::Economy),
                1,
                AttemptOutcome::Completed,
                "task_completed",
            ),
            EscalationDecision::Complete
        );
    }

    #[test]
    fn disposition_is_selected_by_supervisor_policy() {
        assert_eq!(
            disposition_for(EscalationDecision::Retry { profile: None }),
            SupervisorDisposition::Retry
        );
        assert_eq!(
            disposition_for(EscalationDecision::Retry {
                profile: Some(ExecutionProfile::Balanced),
            }),
            SupervisorDisposition::Escalate
        );
        assert_eq!(
            disposition_for(EscalationDecision::Handoff),
            SupervisorDisposition::Handoff
        );
    }

    #[test]
    fn rollups_require_content() {
        let rollup = ResultRollup {
            summary: " ".into(),
            evidence_refs: Value::Null,
        };
        assert!(rollup.summary.trim().is_empty());
    }
}
