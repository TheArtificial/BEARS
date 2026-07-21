//! Durable dispatch/claim/lease state for autonomous `work`-stance execution.
//!
//! One `bear_work_runs` row per dispatch attempt of one job run. The dispatch
//! worker (den-runtime) claims rows with a lease; the BearWire `work.*`
//! methods bind the in-sandbox armature's session and record the turn
//! outcome; the worker harvests and finalizes. Tasks remain the execution
//! checkpoints inside that job run (Docket schedules, gates, and records — it
//! never executes task bodies; ADR-0034).

use serde_json::{json, Value};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

use crate::model::DocketExecutionSessionUpsert;

/// Explicit root name for a provider-managed empty workspace. Absence is not
/// scratch: callers must opt in so rootless dispatch stays invalid.
pub const SCRATCH_ROOT_NAME: &str = "scratch";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkRunState {
    Queued,
    Claimed,
    Provisioning,
    Running,
    Reporting,
    Succeeded,
    Blocked,
    Failed,
    Cancelled,
    TimedOut,
}

impl WorkRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Claimed => "claimed",
            Self::Provisioning => "provisioning",
            Self::Running => "running",
            Self::Reporting => "reporting",
            Self::Succeeded => "succeeded",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "queued" => Self::Queued,
            "claimed" => Self::Claimed,
            "provisioning" => Self::Provisioning,
            "running" => Self::Running,
            "reporting" => Self::Reporting,
            "succeeded" => Self::Succeeded,
            "blocked" => Self::Blocked,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "timed_out" => Self::TimedOut,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Blocked | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

const WORK_RUN_COLUMNS: &str = "id, bear_id, job_id, job_run_id, attempt, state, \
     runner_id, lease_expires_at, cancel_requested, root_name, git_ref, image_name, \
     sandbox_server_url, sandbox_id, sandbox_type, sandbox_strength, work_surface, \
     bearwire_session_id, result_summary, result_refs, usage, error, \
     queued_at, started_at, finished_at, updated_at";

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct WorkRunRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub job_id: Uuid,
    pub job_run_id: Uuid,
    pub attempt: i32,
    pub state: String,
    pub runner_id: Option<String>,
    pub lease_expires_at: Option<OffsetDateTime>,
    pub cancel_requested: bool,
    pub root_name: Option<String>,
    pub git_ref: Option<String>,
    /// Catalog image name the run was dispatched with (None = provider default).
    pub image_name: Option<String>,
    pub sandbox_server_url: Option<String>,
    pub sandbox_id: Option<String>,
    pub sandbox_type: Option<String>,
    pub sandbox_strength: Option<String>,
    pub work_surface: Option<Value>,
    pub bearwire_session_id: Option<String>,
    pub result_summary: Option<String>,
    pub result_refs: Option<Value>,
    pub usage: Option<Value>,
    pub error: Option<String>,
    pub queued_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub updated_at: OffsetDateTime,
}

impl WorkRunRow {
    pub fn state_enum(&self) -> Option<WorkRunState> {
        WorkRunState::parse(&self.state)
    }
}

pub fn effective_work_run_root(
    requested_root: Option<&str>,
    job_work_surface_ref: Option<&str>,
) -> Option<String> {
    requested_root
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            job_work_surface_ref
                .map(str::trim)
                .filter(|name| !name.is_empty())
        })
        .map(ToOwned::to_owned)
}

#[derive(Clone, Debug)]
pub struct WorkJobEnqueue {
    pub bear_id: Uuid,
    pub job_id: Uuid,
    pub root_name: Option<String>,
    pub git_ref: Option<String>,
    pub image_name: Option<String>,
    pub requested_by_user_id: Option<i32>,
}

/// Legacy test fixture input. Production dispatch is job-scoped; tests use a
/// task only to locate its owning job.
#[cfg(test)]
#[derive(Clone, Debug)]
pub struct WorkRunEnqueue {
    pub bear_id: Uuid,
    pub task_id: Uuid,
    pub root_name: Option<String>,
    pub git_ref: Option<String>,
    pub image_name: Option<String>,
    pub requested_by_user_id: Option<i32>,
}

#[cfg(test)]
pub async fn enqueue_work_run(
    pool: &PgPool,
    enqueue: WorkRunEnqueue,
) -> Result<WorkRunRow, DenError> {
    let job_id: Option<(Uuid,)> =
        sqlx::query_as("SELECT job_id FROM bear_tasks WHERE id = $1 AND bear_id = $2")
            .bind(enqueue.task_id)
            .bind(enqueue.bear_id)
            .fetch_optional(pool)
            .await?;
    let (job_id,) = job_id
        .ok_or_else(|| DenError::NotFound(format!("Docket task not found: {}", enqueue.task_id)))?;
    enqueue_work_job(
        pool,
        WorkJobEnqueue {
            bear_id: enqueue.bear_id,
            job_id,
            root_name: enqueue.root_name,
            git_ref: enqueue.git_ref,
            image_name: enqueue.image_name,
            requested_by_user_id: enqueue.requested_by_user_id,
        },
    )
    .await
    .map(|mut runs| runs.remove(0))
}

/// Queue one job-scoped work run. The job must have at least one runnable
/// work task; task state is deliberately not encoded on the work-run row.
pub async fn enqueue_work_job(
    pool: &PgPool,
    enqueue: WorkJobEnqueue,
) -> Result<Vec<WorkRunRow>, DenError> {
    let mut tx = pool.begin().await?;
    let job: Option<(Option<String>, Option<Uuid>, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT work_surface_ref, work_surface_id, current_run_id, status
         FROM bear_jobs WHERE id = $1 AND bear_id = $2 FOR UPDATE",
    )
    .bind(enqueue.job_id)
    .bind(enqueue.bear_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((job_work_surface_ref, _surface_id, current_run_id, _status)) = job else {
        return Err(DenError::NotFound(format!(
            "Docket job not found: {}",
            enqueue.job_id
        )));
    };

    let root_name = effective_work_run_root(
        enqueue.root_name.as_deref(),
        job_work_surface_ref.as_deref(),
    );
    if root_name.is_none() {
        return Err(DenError::ValidationError(
            "no sandbox root configured: choose a root, set work_surface_ref on the job, or explicitly dispatch to scratch".into(),
        ));
    }
    let runnable: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM bear_tasks t
             LEFT JOIN bear_task_run_state s ON s.task_id = t.id AND s.run_id = $2
             WHERE t.job_id = $1 AND t.assigned_to_role = 'work'
               AND COALESCE(s.status, 'pending') IN ('pending', 'blocked')
         )",
    )
    .bind(enqueue.job_id)
    .bind(current_run_id)
    .fetch_one(&mut *tx)
    .await?;
    if !runnable {
        return Err(DenError::ValidationError(
            "job has no runnable work tasks to dispatch".into(),
        ));
    }

    let job_run_id = match current_run_id {
        Some(run_id) => run_id,
        None => {
            let run_id: Uuid = sqlx::query_scalar(
                "INSERT INTO bear_job_runs (job_id, trigger, state) VALUES ($1, 'event', 'running') RETURNING id",
            )
            .bind(enqueue.job_id)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE bear_jobs SET current_run_id = $2, updated_at = now() WHERE id = $1",
            )
            .bind(enqueue.job_id)
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
            run_id
        }
    };
    let attempt: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(attempt), 0) + 1 FROM bear_work_runs WHERE job_id = $1",
    )
    .bind(enqueue.job_id)
    .fetch_one(&mut *tx)
    .await?;
    let run = sqlx::query_as::<_, WorkRunRow>(&format!(
        "INSERT INTO bear_work_runs (bear_id, job_id, job_run_id, attempt, root_name, git_ref, image_name)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(enqueue.bear_id).bind(enqueue.job_id).bind(job_run_id).bind(attempt)
    .bind(root_name).bind(enqueue.git_ref).bind(enqueue.image_name)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| match err {
        sqlx::Error::Database(db) if db.constraint() == Some("idx_bear_work_runs_one_active_per_job") =>
            DenError::ValidationError("job already has an active work run".into()),
        other => other.into(),
    })?;
    tx.commit().await?;
    Ok(vec![run])
}

/// Claim the next dispatchable run with a lease (`FOR UPDATE SKIP LOCKED`).
/// Picks up fresh `queued` runs and takes over non-terminal runs whose lease
/// expired (worker crash); the state of a taken-over run is preserved so the
/// new owner can reconcile rather than restart blindly.
///
/// **Runs serialize per job**: a queued run is only claimable when its job
/// has no other in-flight run, so a multi-task job drains one run at a time
/// in queue order — sequential tasks build on the job's work branch instead
/// of racing it (concurrent publishes to one branch are guaranteed
/// non-fast-forward failures). Expired-lease takeovers are exempt: the
/// in-flight run being taken over *is* the job's active run.
pub async fn claim_next_work_run(
    pool: &PgPool,
    runner_id: &str,
    lease: std::time::Duration,
) -> Result<Option<WorkRunRow>, DenError> {
    // The in-flight-sibling gate reads committed state, so two workers
    // claiming simultaneously can each see the other's queued sibling as
    // claimable. The post-claim recheck resolves that race with a
    // deterministic older-run-wins rule; one bounce is enough because the
    // retry's fresh snapshot sees the winner in flight.
    for _ in 0..3 {
        let Some(run) = claim_next_work_run_once(pool, runner_id, lease).await? else {
            return Ok(None);
        };
        // Only freshly claimed queued runs (no sandbox yet) are subject to
        // the recheck; releasing a provisioned takeover would orphan its
        // sandbox.
        let fresh_claim = run.state == "claimed" && run.sandbox_id.is_none();
        if fresh_claim && has_older_inflight_sibling(pool, &run).await? {
            release_work_run_claim(pool, run.id, runner_id).await?;
            continue;
        }
        return Ok(Some(run));
    }
    Ok(None)
}

async fn claim_next_work_run_once(
    pool: &PgPool,
    runner_id: &str,
    lease: std::time::Duration,
) -> Result<Option<WorkRunRow>, DenError> {
    let lease_secs = i64::try_from(lease.as_secs()).unwrap_or(i64::MAX);
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "WITH candidate AS (
             SELECT id FROM bear_work_runs r
             WHERE (
                     r.state = 'queued'
                     AND NOT EXISTS (
                         SELECT 1 FROM bear_work_runs sibling
                         WHERE sibling.job_id = r.job_id
                           AND sibling.id <> r.id
                           AND sibling.state IN ('claimed', 'provisioning', 'running', 'reporting')
                     )
                   )
                OR (r.state IN ('claimed', 'provisioning', 'running', 'reporting')
                    AND r.lease_expires_at IS NOT NULL AND r.lease_expires_at < now())
             ORDER BY r.queued_at ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE bear_work_runs r
         SET state = CASE WHEN r.state = 'queued' THEN 'claimed' ELSE r.state END,
             runner_id = $1,
             lease_expires_at = now() + make_interval(secs => $2),
             updated_at = now()
         FROM candidate
         WHERE r.id = candidate.id
         RETURNING {}",
        WORK_RUN_COLUMNS
            .split(", ")
            .map(|c| format!("r.{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .bind(runner_id)
    .bind(lease_secs as f64)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Whether the run's job has another in-flight run that was queued earlier
/// (ties broken by id). Used by the claim recheck: when two workers race two
/// queued runs of one job into flight, the younger one yields.
async fn has_older_inflight_sibling(pool: &PgPool, run: &WorkRunRow) -> Result<bool, DenError> {
    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (
             SELECT 1 FROM bear_work_runs sibling
             WHERE sibling.job_id = $1
               AND sibling.id <> $2
               AND sibling.state IN ('claimed', 'provisioning', 'running', 'reporting')
               AND (sibling.queued_at < $3
                    OR (sibling.queued_at = $3 AND sibling.id < $2))
         )",
    )
    .bind(run.job_id)
    .bind(run.id)
    .bind(run.queued_at)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

/// Return a freshly claimed (never provisioned) run to the queue.
async fn release_work_run_claim(
    pool: &PgPool,
    run_id: Uuid,
    runner_id: &str,
) -> Result<(), DenError> {
    sqlx::query(
        "UPDATE bear_work_runs
         SET state = 'queued', runner_id = NULL, lease_expires_at = NULL, updated_at = now()
         WHERE id = $1 AND runner_id = $2 AND state = 'claimed' AND sandbox_id IS NULL",
    )
    .bind(run_id)
    .bind(runner_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Queue placement of a `queued` run within its job (runs serialize per
/// job): 1-based position in the job's queue and the in-flight run it is
/// waiting behind, when there is one. Derived at read time — never stored.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct WorkRunQueueInfo {
    pub run_id: Uuid,
    pub position: i64,
    pub waiting_on_run_id: Option<Uuid>,
}

/// Queue info for the `queued` runs among `run_ids` (non-queued ids are
/// simply absent from the result). One query for the whole batch, so list
/// views can annotate cheaply.
pub async fn queued_run_positions(
    pool: &PgPool,
    run_ids: &[Uuid],
) -> Result<Vec<WorkRunQueueInfo>, DenError> {
    if run_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, WorkRunQueueInfo>(
        "SELECT r.id AS run_id,
                (SELECT count(*) FROM bear_work_runs q
                 WHERE q.job_id = r.job_id AND q.state = 'queued'
                   AND (q.queued_at < r.queued_at
                        OR (q.queued_at = r.queued_at AND q.id <= r.id))) AS position,
                (SELECT s.id FROM bear_work_runs s
                 WHERE s.job_id = r.job_id
                   AND s.state IN ('claimed', 'provisioning', 'running', 'reporting')
                 ORDER BY s.queued_at ASC, s.id ASC
                 LIMIT 1) AS waiting_on_run_id
         FROM bear_work_runs r
         WHERE r.id = ANY($1) AND r.state = 'queued'",
    )
    .bind(run_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// A job-scoped work run that needs attention. Tasks remain visible in the
/// task list; the run is deliberately not attributed to one task.
#[derive(Clone, Debug, serde::Serialize, sqlx::FromRow)]
pub struct AttentionWorkRun {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub job_goal: String,
    pub state: String,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub finished_at: Option<OffsetDateTime>,
}

/// Latest-attempt job runs in attention states. A newer queued/active attempt
/// supersedes an older failure for the same job.
pub async fn attention_work_runs(
    pool: &PgPool,
    bear_id: Uuid,
    job_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<AttentionWorkRun>, DenError> {
    let rows = sqlx::query_as::<_, AttentionWorkRun>(
        "SELECT latest.id AS run_id, latest.job_id, j.goal AS job_goal,
                latest.state, latest.result_summary, latest.error, latest.finished_at
         FROM (
             SELECT DISTINCT ON (job_id)
                    id, job_id, state, result_summary, error, finished_at
             FROM bear_work_runs
             WHERE bear_id = $1 AND ($2::uuid IS NULL OR job_id = $2)
             ORDER BY job_id, queued_at DESC, id DESC
         ) latest
         JOIN bear_jobs j ON j.id = latest.job_id
         WHERE latest.state IN ('blocked', 'failed', 'timed_out')
         ORDER BY latest.finished_at DESC NULLS LAST
         LIMIT $3",
    )
    .bind(bear_id)
    .bind(job_id)
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Jobs whose tasks are all done (or cancelled) in the current run but whose
/// job status has not been closed out — "done but unjudged", awaiting
/// criteria review / completion.
pub async fn jobs_awaiting_completion(
    pool: &PgPool,
    bear_id: Uuid,
) -> Result<Vec<crate::model::DocketJobRow>, DenError> {
    let rows = sqlx::query_as::<_, crate::model::DocketJobRow>(
        "SELECT id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref,
                work_surface_id, commit_policy, work_branch, status, visibility,
                source_conversation_id, objective_kind, current_run_id,
                created_at, updated_at
         FROM bear_jobs j
         WHERE j.bear_id = $1
           AND j.status IN ('ready', 'running')
           AND EXISTS (SELECT 1 FROM bear_tasks t WHERE t.job_id = j.id)
           AND NOT EXISTS (
               SELECT 1 FROM bear_tasks t
               LEFT JOIN bear_task_run_state s
                 ON s.task_id = t.id AND s.run_id = j.current_run_id
               WHERE t.job_id = j.id
                 AND COALESCE(s.status, 'pending') NOT IN ('done', 'cancelled')
           )
         ORDER BY j.updated_at DESC",
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Extend the lease on a run this worker owns. Returns false when the run is
/// no longer owned by `runner_id` (lease was reclaimed) — the worker must
/// drop it.
pub async fn heartbeat_work_run(
    pool: &PgPool,
    run_id: Uuid,
    runner_id: &str,
    lease: std::time::Duration,
) -> Result<bool, DenError> {
    let lease_secs = i64::try_from(lease.as_secs()).unwrap_or(i64::MAX);
    let result = sqlx::query(
        "UPDATE bear_work_runs
         SET lease_expires_at = now() + make_interval(secs => $3), updated_at = now()
         WHERE id = $1 AND runner_id = $2
           AND state IN ('claimed', 'provisioning', 'running', 'reporting')",
    )
    .bind(run_id)
    .bind(runner_id)
    .bind(lease_secs as f64)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

#[derive(Clone, Debug)]
pub struct WorkRunProvisioned {
    pub sandbox_server_url: String,
    pub sandbox_id: String,
    pub sandbox_type: String,
    pub sandbox_strength: String,
    pub work_surface: Value,
}

/// Record sandbox placement and transition claimed → running.
pub async fn record_work_run_provisioned(
    pool: &PgPool,
    run_id: Uuid,
    provisioned: &WorkRunProvisioned,
) -> Result<WorkRunRow, DenError> {
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "UPDATE bear_work_runs
         SET state = 'running',
             sandbox_server_url = $2, sandbox_id = $3, sandbox_type = $4,
             sandbox_strength = $5, work_surface = $6,
             started_at = COALESCE(started_at, now()), updated_at = now()
         WHERE id = $1 AND state IN ('claimed', 'provisioning')
         RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(run_id)
    .bind(&provisioned.sandbox_server_url)
    .bind(&provisioned.sandbox_id)
    .bind(&provisioned.sandbox_type)
    .bind(&provisioned.sandbox_strength)
    .bind(&provisioned.work_surface)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        DenError::ValidationError(format!("work run {run_id} is not in a provisionable state"))
    })?;
    Ok(row)
}

/// Bind the in-sandbox armature's BearWire session to its work run
/// (from `work.checkout`). Fails if the run belongs to a different bear or is
/// not live.
pub async fn bind_work_run_session(
    pool: &PgPool,
    run_id: Uuid,
    bear_id: Uuid,
    session_id: &str,
) -> Result<WorkRunRow, DenError> {
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "UPDATE bear_work_runs
         SET bearwire_session_id = $3, updated_at = now()
         WHERE id = $1 AND bear_id = $2
           AND state IN ('claimed', 'provisioning', 'running')
         RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(run_id)
    .bind(bear_id)
    .bind(session_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        DenError::NotFound(format!(
            "no live work run {run_id} for this bear (wrong id, wrong bear, or already finished)"
        ))
    })?;
    Ok(row)
}

/// Record the Den-side turn outcome for the session bound to a work run and
/// move it to `reporting` (the dispatch worker harvests from there). Returns
/// `None` when the session is not bound to a live run.
pub async fn record_work_run_turn_outcome(
    pool: &PgPool,
    session_id: &str,
    outcome: &Value,
) -> Result<Option<WorkRunRow>, DenError> {
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "UPDATE bear_work_runs
         SET state = 'reporting',
             result_refs = COALESCE(result_refs, '{{}}'::jsonb) || jsonb_build_object('turn_outcome', $2::jsonb),
             updated_at = now()
         WHERE bearwire_session_id = $1 AND state IN ('provisioning', 'running')
         RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(session_id)
    .bind(outcome)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Store the armature's advisory `work.report` summary.
pub async fn record_work_run_report(
    pool: &PgPool,
    run_id: Uuid,
    bear_id: Uuid,
    status_hint: &str,
    summary: &str,
) -> Result<(), DenError> {
    sqlx::query(
        "UPDATE bear_work_runs
         SET result_refs = COALESCE(result_refs, '{}'::jsonb)
                 || jsonb_build_object('armature_report',
                        jsonb_build_object('status_hint', $3::text, 'summary', $4::text)),
             updated_at = now()
         WHERE id = $1 AND bear_id = $2",
    )
    .bind(run_id)
    .bind(bear_id)
    .bind(status_hint)
    .bind(summary)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct WorkRunFinalize {
    pub result_summary: Option<String>,
    /// Merged into (not replacing) existing result_refs.
    pub result_refs: Option<Value>,
    pub usage: Option<Value>,
    pub error: Option<String>,
}

/// Terminal transition; clears the lease, stamps finished_at, and appends the
/// matching task audit event.
pub async fn finalize_work_run(
    pool: &PgPool,
    run_id: Uuid,
    state: WorkRunState,
    finalize: WorkRunFinalize,
) -> Result<WorkRunRow, DenError> {
    if !state.is_terminal() {
        return Err(DenError::ValidationError(format!(
            "finalize_work_run requires a terminal state, got {}",
            state.as_str()
        )));
    }
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "UPDATE bear_work_runs
         SET state = $2,
             result_summary = COALESCE($3, result_summary),
             result_refs = COALESCE(result_refs, '{{}}'::jsonb) || COALESCE($4::jsonb, '{{}}'::jsonb),
             usage = COALESCE($5::jsonb, usage),
             error = COALESCE($6, error),
             runner_id = NULL,
             lease_expires_at = NULL,
             finished_at = COALESCE(finished_at, now()),
             updated_at = now()
         WHERE id = $1 AND state NOT IN ('succeeded', 'blocked', 'failed', 'cancelled', 'timed_out')
         RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(run_id)
    .bind(state.as_str())
    .bind(finalize.result_summary.as_deref())
    .bind(finalize.result_refs.as_ref())
    .bind(finalize.usage.as_ref())
    .bind(finalize.error.as_deref())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        DenError::ValidationError(format!("work run {run_id} is already finalized or unknown"))
    })?;

    let event_type = match state {
        WorkRunState::Succeeded => "completed",
        WorkRunState::Cancelled => "cancelled",
        _ => "blocked",
    };
    let task_ids: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM bear_tasks WHERE job_id = $1 AND assigned_to_role = 'work'")
            .bind(row.job_id)
            .fetch_all(&mut *tx)
            .await?;
    for (task_id,) in task_ids {
        append_task_event(
            &mut tx,
            task_id,
            row.job_run_id,
            event_type,
            None,
            json!({
                "work_run_id": row.id,
                "attempt": row.attempt,
                "final_state": state.as_str(),
                "error": finalize.error,
            }),
        )
        .await?;
    }

    tx.commit().await?;
    Ok(row)
}

/// Ask the owning worker to cancel; teardown happens asynchronously. Returns
/// false when the run is already terminal.
pub async fn request_work_run_cancel(
    pool: &PgPool,
    run_id: Uuid,
    bear_id: Uuid,
) -> Result<bool, DenError> {
    let result = sqlx::query(
        "UPDATE bear_work_runs
         SET cancel_requested = TRUE, updated_at = now()
         WHERE id = $1 AND bear_id = $2
           AND state IN ('queued', 'claimed', 'provisioning', 'running', 'reporting')",
    )
    .bind(run_id)
    .bind(bear_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The live work run bound to a BearWire session, if any. This is the stance
/// signal for `run.start`: a session bound via `work.checkout` runs in the
/// Work stance; everything else stays Pair.
pub async fn get_live_work_run_by_session(
    pool: &PgPool,
    session_id: &str,
) -> Result<Option<WorkRunRow>, DenError> {
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "SELECT {WORK_RUN_COLUMNS} FROM bear_work_runs
         WHERE bearwire_session_id = $1
           AND state IN ('claimed', 'provisioning', 'running', 'reporting')
         ORDER BY updated_at DESC
         LIMIT 1"
    ))
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[derive(Clone, Debug)]
pub struct WorkRunCheckout {
    pub run: WorkRunRow,
    pub prompt: String,
    pub task_title: String,
}

/// The armature side of `work.checkout`: bind the session to its run, open
/// the Docket execution session whose job/task focus satisfies the Work-stance
/// gate, and build the non-interactive prompt from the durable task definition.
pub async fn checkout_work_run_for_session(
    pool: &PgPool,
    run_id: Uuid,
    bear_id: Uuid,
    session_id: &str,
) -> Result<WorkRunCheckout, DenError> {
    let run = bind_work_run_session(pool, run_id, bear_id, session_id).await?;

    let tasks: Vec<(Uuid, String, String, sqlx::types::Json<Vec<String>>)> = sqlx::query_as(
        "SELECT t.id, t.title, t.body, t.completion_criteria
         FROM bear_tasks t
         LEFT JOIN bear_task_run_state s ON s.task_id = t.id AND s.run_id = $2
         WHERE t.job_id = $1 AND t.assigned_to_role = 'work'
           AND COALESCE(s.status, 'pending') IN ('pending', 'blocked')
         ORDER BY t.sibling_order, t.created_at",
    )
    .bind(run.job_id)
    .bind(run.job_run_id)
    .fetch_all(pool)
    .await?;
    if tasks.is_empty() {
        return Err(DenError::ValidationError(
            "job has no runnable work tasks for checkout".into(),
        ));
    }
    let (goal, commit_policy): (String, Option<String>) =
        sqlx::query_as("SELECT goal, commit_policy FROM bear_jobs WHERE id = $1")
            .bind(run.job_id)
            .fetch_one(pool)
            .await?;
    let publishes = matches!(commit_policy.as_deref(), Some("per_task" | "per_job"));

    crate::db::upsert_execution_session(
        pool,
        DocketExecutionSessionUpsert {
            bear_id,
            owner_profile: BearProfile::Work,
            session_id: session_id.to_string(),
            source_conversation_id: None,
            source_client_session_id: Some(session_id.to_string()),
            job_id: run.job_id,
            run_id: run.job_run_id,
            task_id: None,
            state: "active".to_string(),
        },
    )
    .await?;

    let task_title = format!(
        "{} work task{}",
        tasks.len(),
        if tasks.len() == 1 { "" } else { "s" }
    );
    let prompt = build_work_prompt(run.job_id, run.job_run_id, &goal, &tasks, publishes);
    Ok(WorkRunCheckout {
        run,
        prompt,
        task_title,
    })
}

fn build_work_prompt(
    job_id: Uuid,
    run_id: Uuid,
    goal: &str,
    tasks: &[(Uuid, String, String, sqlx::types::Json<Vec<String>>)],
    publishes: bool,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are executing a Docket task autonomously in the work stance, inside a sandbox. \
         No user is present in this session and none will respond — never wait for user input \
         or ask questions.\n\n",
    );
    prompt.push_str(&format!("Job objective: {goal}\n\n"));
    prompt.push_str("Docket execution identifiers:\n");
    prompt.push_str(&format!("- job_id: {job_id}\n"));
    prompt.push_str(&format!("- run_id: {run_id}\n"));
    prompt.push('\n');
    for (task_id, title, body, criteria) in tasks {
        prompt.push_str(&format!("Task ({task_id}): {title}\n"));
        if !body.trim().is_empty() {
            prompt.push_str(&format!("{body}\n"));
        }
        prompt.push_str("Completion criteria — this task is done only when all of these hold:\n");
        for criterion in &criteria.0 {
            prompt.push_str(&format!("- {criterion}\n"));
        }
        prompt.push('\n');
    }
    prompt.push_str(
        "\nRules:\n\
         - Operate only inside the sandbox workspace; it contains the work surface.\n\
         - Work through the listed tasks in order. When each task's criteria are satisfied, \
           call update_current_task_status with its task_id plus the job_id and run_id above, \
           status `done`, and a non-empty result_summary explaining the result.\n\
         - If you cannot make progress on a task, mark that task blocked with a specific reason \
           using update_current_task_status instead of guessing or stopping silently.\n",
    );
    if publishes {
        prompt.push_str(
            "- Commit your work as you go with clear, specific git commit messages; \
               your commits are published to the job's work branch after the run.\n\
             - Do not push, deploy, or call external services; publishing happens \
               outside the sandbox after the run completes.\n",
        );
    } else {
        prompt.push_str("- Do not push, publish, deploy, or call external services.\n");
    }
    prompt
}

pub async fn get_work_run(pool: &PgPool, run_id: Uuid) -> Result<Option<WorkRunRow>, DenError> {
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "SELECT {WORK_RUN_COLUMNS} FROM bear_work_runs WHERE id = $1"
    ))
    .bind(run_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[derive(Clone, Debug, Default)]
pub struct WorkRunListFilter {
    pub bear_id: Option<Uuid>,
    pub job_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub state: Option<String>,
    pub limit: i64,
}

pub async fn list_work_runs(
    pool: &PgPool,
    filter: WorkRunListFilter,
) -> Result<Vec<WorkRunRow>, DenError> {
    let limit = if filter.limit <= 0 {
        50
    } else {
        filter.limit.min(200)
    };
    let rows = sqlx::query_as::<_, WorkRunRow>(&format!(
        "SELECT {WORK_RUN_COLUMNS} FROM bear_work_runs
         WHERE ($1::uuid IS NULL OR bear_id = $1)
           AND ($2::uuid IS NULL OR job_id = $2)
           AND ($3::uuid IS NULL OR task_id = $3)
           AND ($4::text IS NULL OR state = $4)
         ORDER BY queued_at DESC
         LIMIT $5"
    ))
    .bind(filter.bear_id)
    .bind(filter.job_id)
    .bind(filter.task_id)
    .bind(filter.state.as_deref())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Runs currently owned by a worker whose lease is still live, for the
/// monitor step of the dispatch loop.
pub async fn list_owned_work_runs(
    pool: &PgPool,
    runner_id: &str,
) -> Result<Vec<WorkRunRow>, DenError> {
    let rows = sqlx::query_as::<_, WorkRunRow>(&format!(
        "SELECT {WORK_RUN_COLUMNS} FROM bear_work_runs
         WHERE runner_id = $1
           AND state IN ('claimed', 'provisioning', 'running', 'reporting')
         ORDER BY queued_at ASC"
    ))
    .bind(runner_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Merge extra keys into a run's `result_refs` (e.g. the minted armature
/// token id, recorded so a restarted worker can still revoke it).
pub async fn merge_work_run_result_refs(
    pool: &PgPool,
    run_id: Uuid,
    refs: &Value,
) -> Result<(), DenError> {
    sqlx::query(
        "UPDATE bear_work_runs
         SET result_refs = COALESCE(result_refs, '{}'::jsonb) || $2::jsonb, updated_at = now()
         WHERE id = $1",
    )
    .bind(run_id)
    .bind(refs)
    .execute(pool)
    .await?;
    Ok(())
}

/// The run-scoped Docket status of a task (`bear_task_run_state`), used by
/// the harvest step: a work run succeeded only if the model marked the task
/// done in-turn. `None` = no run state recorded (treated as pending).
pub async fn get_task_run_status(
    pool: &PgPool,
    job_run_id: Uuid,
    task_id: Uuid,
) -> Result<Option<String>, DenError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT status FROM bear_task_run_state WHERE run_id = $1 AND task_id = $2")
            .bind(job_run_id)
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(status,)| status))
}

/// Run-scoped states for every work task in a job. A job-scoped work run
/// owns the sandbox lifecycle; these remain the individual completion
/// checkpoints reported by the agent.
pub async fn get_job_work_task_run_statuses(
    pool: &PgPool,
    job_id: Uuid,
    job_run_id: Uuid,
) -> Result<Vec<(Uuid, String)>, DenError> {
    sqlx::query_as(
        "SELECT t.id, COALESCE(s.status, 'pending')
         FROM bear_tasks t
         LEFT JOIN bear_task_run_state s ON s.task_id = t.id AND s.run_id = $2
         WHERE t.job_id = $1 AND t.assigned_to_role = 'work'
         ORDER BY t.sibling_order, t.created_at",
    )
    .bind(job_id)
    .bind(job_run_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Close the work-stance execution session opened by `work.checkout`.
pub async fn close_work_execution_session(
    pool: &PgPool,
    bear_id: Uuid,
    session_id: &str,
) -> Result<(), DenError> {
    sqlx::query(
        "UPDATE docket_execution_sessions
         SET state = 'completed', updated_at = now()
         WHERE bear_id = $1 AND owner_profile = 'work' AND session_id = $2
           AND state IN ('active', 'blocked', 'completing', 'paused')",
    )
    .bind(bear_id)
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Bears that have any work-assigned tasks, for the optional auto-enqueue
/// sweep (`WORK_DISPATCH_AUTO`).
pub async fn list_bears_with_work_tasks(pool: &PgPool) -> Result<Vec<Uuid>, DenError> {
    let rows: Vec<(Uuid,)> =
        sqlx::query_as("SELECT DISTINCT bear_id FROM bear_tasks WHERE assigned_to_role = 'work'")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Everything the dispatch worker needs to provision a claimed run.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct WorkRunDispatchContext {
    pub bear_slug: String,
    pub bear_name: String,
    pub created_by_user_id: i32,
    pub job_goal: String,
    pub work_surface_ref: Option<String>,
    pub commit_policy: Option<String>,
    pub work_branch: Option<String>,
    pub allow_default_ref: bool,
}

impl WorkRunDispatchContext {
    /// Whether the job's commit policy publishes successful runs to the
    /// upstream work branch (`per_task` / `per_job`; `propose_only`/`none`
    /// stay diff-only).
    pub fn publishes(&self) -> bool {
        matches!(self.commit_policy.as_deref(), Some("per_task" | "per_job"))
    }
}

pub async fn get_work_run_dispatch_context(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<WorkRunDispatchContext, DenError> {
    sqlx::query_as::<_, WorkRunDispatchContext>(
        "SELECT b.slug AS bear_slug, b.name AS bear_name, j.created_by_user_id, j.goal AS job_goal, j.work_surface_ref,
                j.commit_policy, j.work_branch,
                COALESCE(j.work_branch = s.default_ref, FALSE) AS allow_default_ref
         FROM bear_work_runs r
         JOIN bears b ON b.id = r.bear_id
         JOIN bear_jobs j ON j.id = r.job_id
         LEFT JOIN work_surfaces s ON s.id = j.work_surface_id
         WHERE r.id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DenError::NotFound(format!("work run not found: {run_id}")))
}

/// The branch a job's work runs publish to, setting the generated default
/// (`den/job-<short-id>`) on first use. Callers may have set an explicit
/// branch at job creation; this never overwrites one.
pub async fn ensure_job_work_branch(pool: &PgPool, job_id: Uuid) -> Result<String, DenError> {
    let generated = format!("den/job-{}", &job_id.simple().to_string()[..8]);
    let (branch,): (String,) = sqlx::query_as(
        "UPDATE bear_jobs
         SET work_branch = COALESCE(work_branch, $2), updated_at = now()
         WHERE id = $1
         RETURNING work_branch",
    )
    .bind(job_id)
    .bind(&generated)
    .fetch_one(pool)
    .await?;
    Ok(branch)
}

async fn append_task_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task_id: Uuid,
    job_run_id: Uuid,
    event_type: &str,
    by_user_id: Option<i32>,
    payload: Value,
) -> Result<(), DenError> {
    sqlx::query(
        "INSERT INTO bear_task_events (task_id, run_id, event_type, by_role, by_user_id, payload)
         VALUES ($1, $2, $3, 'work', $4, $5::jsonb)",
    )
    .bind(task_id)
    .bind(job_run_id)
    .bind(event_type)
    .bind(by_user_id)
    .bind(&payload)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::effective_work_run_root;

    #[test]
    fn effective_work_run_root_prefers_trimmed_request_then_job_default() {
        assert_eq!(
            effective_work_run_root(Some(" requested "), Some("job-default")),
            Some("requested".to_string())
        );
        assert_eq!(
            effective_work_run_root(Some("   "), Some(" job-default ")),
            Some("job-default".to_string())
        );
        assert_eq!(effective_work_run_root(None, Some("   ")), None);
    }
}
