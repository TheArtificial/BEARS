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

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticOutcome {
    pub outcome: String,
    pub cause: Option<String>,
    pub disposition: Option<String>,
    pub evidence: Option<Value>,
    pub recovery_action: Option<String>,
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
    // sqlx-dynamic: transitional static query; this module is included in the checked-query ratchet and will migrate when workspace SQLx metadata is refreshed.
    let (job_id, job_status, run_state): (Uuid, String, String) = sqlx::query_as(
        "SELECT j.id, COALESCE(j.lifecycle_intent, 'derived'), r.state FROM bear_job_runs r JOIN bear_jobs j ON j.id=r.job_id WHERE r.id=$1",
    ).bind(run_id).fetch_optional(pool).await?
        .ok_or_else(|| DenError::NotFound(format!("Docket run not found: {run_id}")))?;

    // Cursors are intentionally absent: current execution comes only from run state.
    // sqlx-dynamic: transitional static query; see module ratchet note above.
    let current_task = sqlx::query_as::<_, DiagnosticTask>(
        "SELECT t.id, t.title, s.status FROM bear_task_run_state s JOIN bear_tasks t ON t.id=s.task_id WHERE s.run_id=$1 AND s.status='in_progress'",
    ).bind(run_id).fetch_optional(pool).await?;
    // sqlx-dynamic: transitional static query; see module ratchet note above.
    let states: Vec<String> = sqlx::query_scalar(
        "SELECT COALESCE(s.status, 'pending') FROM bear_tasks t LEFT JOIN bear_task_run_state s ON s.task_id=t.id AND s.run_id=$1 WHERE t.job_id=$2 ORDER BY t.sibling_order, t.id",
    ).bind(run_id).bind(job_id).fetch_all(pool).await?;

    // sqlx-dynamic: transitional static query; see module ratchet note above.
    let attention = sqlx::query_as::<_, DiagnosticAttention>(
        "SELECT task_id, cause_code AS cause, recovery_action, evidence_refs AS evidence, created_at FROM docket_attention WHERE run_id=$1 AND resolved_at IS NULL",
    ).bind(run_id).fetch_optional(pool).await?;
    // sqlx-dynamic: transitional static query; see module ratchet note above.
    let attachment = sqlx::query_as::<_, DiagnosticAttachment>(
        "SELECT execution_target, attached_client_session_id AS client_session_id,
                attachment_state AS state, attachment_warning AS warning,
                disconnected_at, disconnect_deadline_at AS deadline_at,
                result_refs #>> '{recovery,source_work_run_id}' AS source_work_run_id,
                (execution_target = 'attached_armature' AND state = 'timed_out'
                 AND result_refs #>> '{outcome,code}' = 'armature_disconnect_timeout') AS recovery_eligible
         FROM bear_work_runs WHERE job_run_id=$1 ORDER BY attempt DESC LIMIT 1",
    ).bind(run_id).fetch_optional(pool).await?;
    // sqlx-dynamic: transitional static query; see module ratchet note above.
    let rollups = sqlx::query_as::<_, DiagnosticRollup>(
        "SELECT task_id, parent_task_id, summary, evidence_refs AS evidence, created_at FROM docket_result_rollups WHERE run_id=$1 ORDER BY created_at, task_id",
    ).bind(run_id).fetch_all(pool).await?;
    // sqlx-dynamic: transitional static query; see module ratchet note above.
    let routes = sqlx::query_as::<_, RoutingEventRow>(
        "SELECT created_at, task_id, conversation_strategy, resolved_profile, reason FROM docket_routing_decisions WHERE run_id=$1 ORDER BY created_at, id",
    ).bind(run_id).fetch_all(pool).await?;
    // sqlx-dynamic: transitional static query; see module ratchet note above.
    let attempts = sqlx::query_as::<_, AttemptEventRow>(
        "SELECT a.started_at, a.finished_at, d.task_id, a.attempt, a.outcome, a.cause_code, a.retry_disposition, a.evidence_refs, a.resolved_profile, a.latency_ms, a.cost_microusd FROM docket_turn_attempts a JOIN docket_routing_decisions d ON d.id=a.routing_decision_id WHERE d.run_id=$1 ORDER BY a.started_at, a.id",
    ).bind(run_id).fetch_all(pool).await?;

    let explanation = explain_state(&job_status, &run_state, &states, attention.as_ref());
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
                outcome: Some(DiagnosticOutcome {
                    outcome,
                    cause: attempt.cause_code,
                    disposition: attempt.retry_disposition,
                    evidence: attempt.evidence_refs,
                    recovery_action: attention.as_ref().map(|a| a.recovery_action.clone()),
                }),
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
        attention,
        attachment,
        rollups,
        timeline,
    })
}

fn explain_state(
    job_status: &str,
    run_state: &str,
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
    if states.iter().any(|state| state == "in_progress") {
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
    fn explanation_never_treats_empty_queue_as_completion() {
        assert!(
            explain_state("running", "running", &["blocked".into()], None).starts_with("Blocked")
        );
        assert!(
            explain_state("completed", "completed", &["done".into()], None)
                .starts_with("Completed")
        );
    }
}
