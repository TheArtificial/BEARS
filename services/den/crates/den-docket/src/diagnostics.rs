//! Canonical read-only job-run diagnostics for conversation and web surfaces.

use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

#[derive(Clone, Debug, Serialize)]
pub struct RunDiagnostics {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub job_status: String,
    pub run_state: String,
    pub current_task: Option<DiagnosticTask>,
    pub explanation: String,
    /// The latest canonical failure/recovery truth, suitable for conversation
    /// headlines as well as richer forensic views.
    pub failure: Option<NormalizedFailure>,
    pub attention: Option<DiagnosticAttention>,
    pub attachment: Option<DiagnosticAttachment>,
    pub rollups: Vec<DiagnosticRollup>,
    pub timeline: Vec<DiagnosticEvent>,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct DiagnosticTask {
    pub id: Uuid,
    pub title: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct DiagnosticAttention {
    pub task_id: Option<Uuid>,
    pub cause: String,
    pub recovery_action: String,
    pub evidence: Value,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct DiagnosticAttachment {
    pub execution_target: String,
    pub client_session_id: Option<String>,
    pub state: Option<String>,
    pub warning: Option<String>,
    pub disconnected_at: Option<OffsetDateTime>,
    pub deadline_at: Option<OffsetDateTime>,
    pub source_work_run_id: Option<String>,
    pub recovery_eligible: bool,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct DiagnosticRollup {
    pub task_id: Uuid,
    pub parent_task_id: Uuid,
    pub summary: String,
    pub evidence: Value,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticEvent {
    pub at: OffsetDateTime,
    pub kind: String,
    pub task_id: Option<Uuid>,
    pub summary: String,
    pub outcome: Option<DiagnosticOutcome>,
}

/// Canonical failure truth shared by concise status and forensic timeline views.
///
/// The raw outcome remains available on [`DiagnosticOutcome`] for compatibility,
/// but consumers should render this shape rather than infer recovery semantics
/// from individual attempt fields.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct NormalizedFailure {
    pub outcome: String,
    pub cause: Option<String>,
    pub disposition: Option<String>,
    pub evidence: Option<Value>,
    pub recovery_action: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticOutcome {
    pub outcome: String,
    pub cause: Option<String>,
    pub disposition: Option<String>,
    pub evidence: Option<Value>,
    pub recovery_action: Option<String>,
}

impl From<NormalizedFailure> for DiagnosticOutcome {
    fn from(failure: NormalizedFailure) -> Self {
        Self {
            outcome: failure.outcome,
            cause: failure.cause,
            disposition: failure.disposition,
            evidence: failure.evidence,
            recovery_action: failure.recovery_action,
        }
    }
}

#[derive(sqlx::FromRow)]
struct RoutingEventRow {
    created_at: OffsetDateTime,
    task_id: Uuid,
    conversation_strategy: String,
    resolved_profile: Option<String>,
    reason: String,
}

#[derive(sqlx::FromRow)]
struct AttemptEventRow {
    started_at: OffsetDateTime,
    finished_at: Option<OffsetDateTime>,
    task_id: Uuid,
    attempt: i32,
    outcome: Option<String>,
    cause_code: Option<String>,
    retry_disposition: Option<String>,
    evidence_refs: Option<Value>,
    resolved_profile: Option<String>,
    latency_ms: Option<i64>,
    cost_microusd: Option<i64>,
}

pub async fn run_diagnostics(pool: &PgPool, run_id: Uuid) -> Result<RunDiagnostics, DenError> {
    let run = sqlx::query!(
        r#"
        SELECT
            j.id AS "job_id!: Uuid",
            COALESCE(j.lifecycle_intent, 'derived') AS "job_status!: String",
            r.state AS "run_state!: String"
        FROM bear_job_runs r
        JOIN bear_jobs j ON j.id = r.job_id
        WHERE r.id = $1
        "#,
        run_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DenError::NotFound(format!("Docket run not found: {run_id}")))?;
    let job_id = run.job_id;
    let job_status = run.job_status;
    let run_state = run.run_state;

    // Cursors are intentionally absent: current execution comes only from run state.
    let current_task = sqlx::query_as!(
        DiagnosticTask,
        r#"
        SELECT
            t.id AS "id!: Uuid",
            t.title AS "title!: String",
            'in_progress' AS "status!: String"
        FROM bear_work_runs w
        JOIN bear_tasks t ON t.id = w.executing_task_id
        WHERE w.job_run_id = $1
          AND w.state IN ('claimed', 'provisioning', 'running', 'paused', 'reporting')
        ORDER BY w.attempt DESC
        LIMIT 1
        "#,
        run_id
    )
    .fetch_optional(pool)
    .await?;
    let states: Vec<String> = sqlx::query_scalar!(
        r#"
        SELECT COALESCE(s.status, 'pending') AS "status!: String"
        FROM bear_tasks t
        LEFT JOIN bear_task_run_state s ON s.task_id = t.id AND s.run_id = $1
        WHERE t.job_id = $2
        ORDER BY t.sibling_order, t.id
        "#,
        run_id,
        job_id
    )
    .fetch_all(pool)
    .await?;

    let attention = sqlx::query_as!(
        DiagnosticAttention,
        r#"
        SELECT
            task_id AS "task_id?: Uuid",
            cause_code AS "cause!: String",
            recovery_action AS "recovery_action!: String",
            evidence_refs AS "evidence!: Value",
            created_at AS "created_at!: OffsetDateTime"
        FROM docket_attention
        WHERE run_id = $1 AND resolved_at IS NULL
        "#,
        run_id
    )
    .fetch_optional(pool)
    .await?;
    let attachment = sqlx::query_as!(
        DiagnosticAttachment,
        r#"
        SELECT
            execution_target AS "execution_target!: String",
            attached_client_session_id AS "client_session_id?: String",
            attachment_state AS "state?: String",
            attachment_warning AS "warning?: String",
            disconnected_at AS "disconnected_at?: OffsetDateTime",
            disconnect_deadline_at AS "deadline_at?: OffsetDateTime",
            result_refs #>> '{recovery,source_work_run_id}' AS "source_work_run_id?: String",
            (execution_target = 'attached_armature' AND attachment_state = 'timed_out'
             AND result_refs #>> '{outcome,code}' = 'armature_disconnect_timeout')
                AS "recovery_eligible!: bool"
        FROM bear_work_runs
        WHERE job_run_id = $1
        ORDER BY attempt DESC
        LIMIT 1
        "#,
        run_id
    )
    .fetch_optional(pool)
    .await?;
    let rollups = sqlx::query_as!(
        DiagnosticRollup,
        r#"
        SELECT
            task_id AS "task_id!: Uuid",
            parent_task_id AS "parent_task_id!: Uuid",
            summary AS "summary!: String",
            evidence_refs AS "evidence!: Value",
            created_at AS "created_at!: OffsetDateTime"
        FROM docket_result_rollups
        WHERE run_id = $1
        ORDER BY created_at, task_id
        "#,
        run_id
    )
    .fetch_all(pool)
    .await?;
    let routes = sqlx::query_as!(
        RoutingEventRow,
        r#"
        SELECT
            created_at AS "created_at!: OffsetDateTime",
            task_id AS "task_id!: Uuid",
            conversation_strategy AS "conversation_strategy!: String",
            resolved_profile AS "resolved_profile?: String",
            reason AS "reason!: String"
        FROM docket_routing_decisions
        WHERE run_id = $1
        ORDER BY created_at, id
        "#,
        run_id
    )
    .fetch_all(pool)
    .await?;
    let attempts = sqlx::query_as!(
        AttemptEventRow,
        r#"
        SELECT
            a.started_at AS "started_at!: OffsetDateTime",
            a.finished_at AS "finished_at?: OffsetDateTime",
            d.task_id AS "task_id!: Uuid",
            a.attempt AS "attempt!: i32",
            a.outcome AS "outcome?: String",
            a.cause_code AS "cause_code?: String",
            a.retry_disposition AS "retry_disposition?: String",
            a.evidence_refs AS "evidence_refs?: Value",
            a.resolved_profile AS "resolved_profile?: String",
            a.latency_ms AS "latency_ms?: i64",
            a.cost_microusd AS "cost_microusd?: i64"
        FROM docket_turn_attempts a
        JOIN docket_routing_decisions d ON d.id = a.routing_decision_id
        WHERE d.run_id = $1
        ORDER BY a.started_at, a.id
        "#,
        run_id
    )
    .fetch_all(pool)
    .await?;

    let explanation = explain_state(
        &job_status,
        &run_state,
        current_task.is_some(),
        &states,
        attention.as_ref(),
    );
    let mut failure = attention.as_ref().map(|attention| NormalizedFailure {
        outcome: "attention_required".into(),
        cause: Some(attention.cause.clone()),
        disposition: None,
        evidence: Some(attention.evidence.clone()),
        recovery_action: Some(attention.recovery_action.clone()),
    });
    let mut timeline = Vec::with_capacity(routes.len() + attempts.len() * 2 + rollups.len());
    for route in routes {
        timeline.push(DiagnosticEvent {
            at: route.created_at,
            kind: "routing_decided".into(),
            task_id: Some(route.task_id),
            summary: format!(
                "{}: {} ({})",
                route.conversation_strategy,
                route.reason,
                route
                    .resolved_profile
                    .as_deref()
                    .unwrap_or("conversation fallback")
            ),
            outcome: None,
        });
    }
    for attempt in attempts {
        timeline.push(DiagnosticEvent {
            at: attempt.started_at,
            kind: "attempt_started".into(),
            task_id: Some(attempt.task_id),
            summary: format!(
                "attempt {} using {}",
                attempt.attempt,
                attempt
                    .resolved_profile
                    .as_deref()
                    .unwrap_or("conversation fallback")
            ),
            outcome: None,
        });
        if let (Some(at), Some(outcome)) = (attempt.finished_at, attempt.outcome) {
            let normalized = NormalizedFailure {
                outcome: outcome.clone(),
                cause: attempt.cause_code.clone(),
                disposition: attempt.retry_disposition.clone(),
                evidence: attempt.evidence_refs.clone(),
                recovery_action: attention.as_ref().map(|a| a.recovery_action.clone()),
            };
            if normalized.cause.is_some() || normalized.disposition.is_some() {
                failure = Some(normalized.clone());
            }
            let attribution = match (attempt.latency_ms, attempt.cost_microusd) {
                (None, None) => String::new(),
                (latency, cost) => format!(
                    "; latency={}ms cost={}µUSD",
                    latency.map_or_else(|| "unknown".into(), |v| v.to_string()),
                    cost.map_or_else(|| "unknown".into(), |v| v.to_string())
                ),
            };
            timeline.push(DiagnosticEvent {
                at,
                kind: "attempt_terminal".into(),
                task_id: Some(attempt.task_id),
                summary: format!("attempt {}: {}{}", attempt.attempt, outcome, attribution),
                outcome: Some(normalized.into()),
            });
        }
    }
    for rollup in &rollups {
        timeline.push(DiagnosticEvent {
            at: rollup.created_at,
            kind: "rollup_persisted".into(),
            task_id: Some(rollup.task_id),
            summary: rollup.summary.clone(),
            outcome: None,
        });
    }
    timeline.sort_by_key(|event| event.at);
    Ok(RunDiagnostics {
        job_id,
        run_id,
        job_status,
        run_state,
        current_task,
        explanation,
        failure,
        attention,
        attachment,
        rollups,
        timeline,
    })
}

fn explain_state(
    job_status: &str,
    run_state: &str,
    has_active_task: bool,
    states: &[String],
    attention: Option<&DiagnosticAttention>,
) -> String {
    if job_status == "completed" || run_state == "completed" {
        return "Completed because every non-cancelled task is terminal and all completion criteria are satisfied.".into();
    }
    if let Some(attention) = attention {
        return format!(
            "Blocked because {}. Recovery: {}.",
            attention.cause, attention.recovery_action
        );
    }
    if has_active_task {
        "Running the current authoritative task; browsing cursors do not affect execution.".into()
    } else if states.iter().any(|state| state == "pending") {
        "Ready because at least one task remains actionable.".into()
    } else {
        "Blocked because work remains incomplete and no task is currently actionable.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_failure_preserves_recovery_semantics_for_timeline_rendering() {
        let failure = NormalizedFailure {
            outcome: "failed".into(),
            cause: Some("watchdog_timeout".into()),
            disposition: Some("retry".into()),
            evidence: Some(serde_json::json!({"attempt": 2})),
            recovery_action: Some("retry_task".into()),
        };

        let timeline: DiagnosticOutcome = failure.clone().into();
        assert_eq!(timeline.outcome, failure.outcome);
        assert_eq!(timeline.cause, failure.cause);
        assert_eq!(timeline.recovery_action, failure.recovery_action);
    }

    #[test]
    fn explanation_never_treats_empty_queue_as_completion() {
        assert!(
            explain_state("running", "running", false, &["blocked".into()], None)
                .starts_with("Blocked")
        );
        assert!(
            explain_state("completed", "completed", false, &["done".into()], None)
                .starts_with("Completed")
        );
    }
}
