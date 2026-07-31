//! Durable dispatch/claim/lease state for autonomous `work`-stance execution.
//!
//! One `bear_work_runs` row per dispatch attempt of one job run. The dispatch
//! worker (den-runtime) claims rows with a lease; the BearWire `work.*`
//! methods bind the in-sandbox armature's session and record the turn
//! outcome; the worker harvests and finalizes. Tasks remain the execution
//! checkpoints inside that job run (Docket schedules, gates, and records — it
//! never executes task bodies; ADR-0034).

use std::time::Duration as StdDuration;

use serde_json::{json, Value};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

use crate::dispatcher::TaskDispatcher;
use crate::execution_profiles::resolve_execution_profile;
use crate::model::{DocketExecutionSessionUpsert, DocketTaskDifficulty};
use crate::recovery::start_turn_attempt;
use crate::routing::{route_turn, ExecutionSurface, TurnIntent, TurnSource};
use crate::service::PgDocketService;

/// Explicit root name for a provider-managed empty workspace. Absence is not
/// scratch: callers must opt in so rootless dispatch stays invalid.
pub const SCRATCH_ROOT_NAME: &str = "scratch";
pub const ATTACHED_DISCONNECT_TIMEOUT: StdDuration = StdDuration::from_mins(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkExecutionTarget {
    Sandbox,
    AttachedArmature { client_session_id: String },
}

impl WorkExecutionTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::AttachedArmature { .. } => "attached_armature",
        }
    }

    fn client_session_id(&self) -> Option<&str> {
        match self {
            Self::Sandbox => None,
            Self::AttachedArmature { client_session_id } => Some(client_session_id),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkRunState {
    Queued,
    Claimed,
    Provisioning,
    Running,
    Paused,
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
            Self::Paused => "paused",
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
            "paused" => Self::Paused,
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
     runner_id, lease_expires_at, cancel_requested, cancel_requested_by, cancel_reason, \
     cancel_requested_at, root_name, git_ref, image_name, \
     sandbox_server_url, sandbox_id, sandbox_type, sandbox_strength, work_surface, \
     execution_target, attached_client_session_id, attachment_state, attachment_warning, \
     disconnected_at, disconnect_deadline_at, \
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
    /// Origin that requested cancellation, e.g. `web:user:42` or `tool:pair`.
    pub cancel_requested_by: Option<String>,
    /// Caller-provided reason suitable for a diagnostic surface, never a secret.
    pub cancel_reason: Option<String>,
    pub cancel_requested_at: Option<OffsetDateTime>,
    pub root_name: Option<String>,
    pub git_ref: Option<String>,
    /// Catalog image name the run was dispatched with (None = provider default).
    pub image_name: Option<String>,
    pub sandbox_server_url: Option<String>,
    pub sandbox_id: Option<String>,
    pub sandbox_type: Option<String>,
    pub sandbox_strength: Option<String>,
    pub work_surface: Option<Value>,
    pub execution_target: String,
    pub attached_client_session_id: Option<String>,
    pub attachment_state: Option<String>,
    pub attachment_warning: Option<String>,
    pub disconnected_at: Option<OffsetDateTime>,
    pub disconnect_deadline_at: Option<OffsetDateTime>,
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
    managed_surface_name: Option<&str>,
) -> Option<String> {
    requested_root
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            managed_surface_name
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
    pub execution_target: WorkExecutionTarget,
    pub attachment_warning: Option<String>,
}

/// Provenance captured when a user-facing control plane asks a worker to stop.
#[derive(Clone, Debug)]
pub struct WorkRunCancelRequest {
    pub requested_by: String,
    pub reason: String,
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
            execution_target: WorkExecutionTarget::Sandbox,
            attachment_warning: None,
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
    type JobEnqueueRow = (
        Option<Uuid>,
        Option<String>,
        Option<Uuid>,
        Option<String>,
        bool,
    );
    let job: Option<JobEnqueueRow> = sqlx::query_as(
        "SELECT j.work_surface_id, s.name, j.current_run_id, j.status,
                    EXISTS (
                        SELECT 1 FROM work_surface_bears wsb
                        WHERE wsb.surface_id = j.work_surface_id AND wsb.bear_id = j.bear_id
                    ) AS surface_assigned
             FROM bear_jobs j
             LEFT JOIN work_surfaces s ON s.id = j.work_surface_id
             WHERE j.id = $1 AND j.bear_id = $2 FOR UPDATE OF j",
    )
    .bind(enqueue.job_id)
    .bind(enqueue.bear_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((surface_id, surface_name, current_run_id, status, surface_assigned)) = job else {
        return Err(DenError::NotFound(format!(
            "Docket job not found: {}",
            enqueue.job_id
        )));
    };
    if surface_id.is_none()
        || surface_name
            .as_deref()
            .is_none_or(|name| name.trim().is_empty())
    {
        return Err(DenError::ValidationError(
            "work_surface_required: this work job lacks a valid managed work-surface binding; select or rebind a surface before dispatch".into(),
        ));
    }
    if !surface_assigned {
        return Err(DenError::ValidationError(format!(
            "managed work surface '{}' is not assigned to this Bear",
            surface_name.as_deref().unwrap_or("unknown")
        )));
    }
    if !matches!(status.as_deref(), Some("ready") | Some("running")) {
        return Err(DenError::ValidationError(
            "job is not dispatchable; only ready or running work jobs can start work runs".into(),
        ));
    }

    let root_name = effective_work_run_root(enqueue.root_name.as_deref(), surface_name.as_deref());
    if root_name.is_none() {
        return Err(DenError::ValidationError(
            "no sandbox root configured: choose a root, select a managed surface on the job, or explicitly dispatch to scratch".into(),
        ));
    }
    let runnable: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM bear_tasks t
             LEFT JOIN bear_task_run_state s ON s.task_id = t.id AND s.run_id = $2
             WHERE t.job_id = $1
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
    let execution_target = enqueue.execution_target.as_str();
    let attached_client_session_id = enqueue.execution_target.client_session_id();
    let attachment_state = attached_client_session_id.map(|_| "attached");
    let run = sqlx::query_as::<_, WorkRunRow>(&format!(
        "INSERT INTO bear_work_runs (bear_id, job_id, job_run_id, attempt, root_name, git_ref, image_name,
                                     execution_target, attached_client_session_id, attachment_state,
                                     attachment_warning)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(enqueue.bear_id).bind(enqueue.job_id).bind(job_run_id).bind(attempt)
    .bind(root_name).bind(enqueue.git_ref).bind(enqueue.image_name)
    .bind(execution_target).bind(attached_client_session_id).bind(attachment_state)
    .bind(enqueue.attachment_warning)
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
                     AND r.execution_target = 'sandbox'
                     AND EXISTS (
                         SELECT 1 FROM bear_jobs j
                         WHERE j.id = r.job_id AND j.status IN ('ready', 'running')
                     )
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
        "SELECT id, bear_id, created_by_user_id, created_by_role, goal, work_surface_id, commit_policy, work_branch, status, visibility,
                source_conversation_id, objective_kind, current_run_id, supersedes_job_id,
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
    pub rust_dependency_preparation: Option<Value>,
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
             result_refs = CASE
                 WHEN $7::jsonb IS NULL THEN result_refs
                 ELSE COALESCE(result_refs, '{{}}'::jsonb)
                      || jsonb_build_object('rust_dependency_preparation', $7::jsonb)
             END,
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
    .bind(&provisioned.rust_dependency_preparation)
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

/// Store the latest hosted Cargo dependency-preparation result for a work run.
/// This is durable run evidence for the web UI; it deliberately excludes any
/// provider credentials or unbounded helper output.
pub async fn record_work_run_dependency_preparation(
    pool: &PgPool,
    run_id: Uuid,
    bear_id: Uuid,
    result: &Value,
) -> Result<(), DenError> {
    sqlx::query(
        "UPDATE bear_work_runs
         SET result_refs = COALESCE(result_refs, '{}'::jsonb)
                 || jsonb_build_object('rust_dependency_preparation', $3::jsonb),
             updated_at = now()
         WHERE id = $1 AND bear_id = $2",
    )
    .bind(run_id)
    .bind(bear_id)
    .bind(result)
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

/// The one user-facing outcome for a work run. Turn and Armature reports are
/// evidence only: neither may claim that work completed. A completed worker
/// process is only eligible for a completed work outcome; durable task state
/// and structured validation evidence decide the outcome.
pub fn canonical_work_run_state(
    state: WorkRunState,
    refs: &Value,
    task_statuses: &[String],
) -> WorkRunState {
    if !matches!(state, WorkRunState::Succeeded) {
        return state;
    }
    if refs.pointer("/cargo_failure/code").and_then(Value::as_str)
        == Some("cargo_offline_cache_miss")
        || task_statuses.iter().any(|status| status == "blocked")
        || task_statuses
            .iter()
            .any(|status| matches!(status.as_str(), "pending" | "in_progress"))
    {
        WorkRunState::Blocked
    } else {
        state
    }
}

pub fn canonical_work_run_outcome(
    state: WorkRunState,
    refs: &Value,
    task_statuses: &[String],
) -> Value {
    if matches!(state, WorkRunState::Blocked)
        && refs.pointer("/cargo_failure/code").and_then(Value::as_str)
            == Some("cargo_offline_cache_miss")
    {
        let package = refs
            .pointer("/cargo_failure/required_package")
            .and_then(Value::as_str)
            .map(|package| format!(" `{package}` could not be resolved."))
            .unwrap_or_default();
        return json!({
            "status": "blocked",
            "code": "cargo_offline_cache_miss",
            "summary": format!("Rust dependencies are unavailable in the offline cache.{package}"),
            "next_action": "Prepare Rust dependencies with the hosted dependency tool, then retry Cargo.",
            "evidence_refs": ["cargo_failure", "turn_outcome", "armature_report"],
        });
    }

    if matches!(state, WorkRunState::Blocked) {
        let blocked = task_statuses
            .iter()
            .filter(|status| *status == "blocked")
            .count();
        if blocked > 0 {
            return json!({
                "status": "blocked",
                "code": "task_blocked",
                "summary": format!("Work is blocked: {blocked} task(s) are blocked."),
                "evidence_refs": ["task_run_states", "turn_outcome", "armature_report"],
            });
        }
        let unfinished = task_statuses
            .iter()
            .filter(|status| matches!(status.as_str(), "pending" | "in_progress"))
            .count();
        if unfinished > 0 {
            return json!({
                "status": "incomplete",
                "code": "work_incomplete",
                "summary": format!("Work is incomplete: {unfinished} task(s) remain unfinished."),
                "evidence_refs": ["task_run_states", "turn_outcome", "armature_report"],
            });
        }
    }

    let (status, code, summary) = match state {
        WorkRunState::Succeeded => ("completed", "completed", "Work completed."),
        WorkRunState::Blocked => ("blocked", "work_blocked", "Work is blocked."),
        WorkRunState::Failed => ("failed", "work_failed", "Work failed."),
        WorkRunState::TimedOut => ("timed_out", "work_timed_out", "Work timed out."),
        WorkRunState::Cancelled => ("cancelled", "work_cancelled", "Work was cancelled."),
        _ => ("incomplete", "work_incomplete", "Work is incomplete."),
    };
    json!({
        "status": status,
        "code": code,
        "summary": summary,
        "evidence_refs": ["turn_outcome", "armature_report"],
    })
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
    let task_statuses: Vec<(String,)> = sqlx::query_as(
        "SELECT COALESCE(s.status, 'pending')
         FROM bear_tasks t
         LEFT JOIN bear_task_run_state s ON s.task_id = t.id AND s.run_id = $2
         WHERE t.job_id = $1",
    )
    .bind(row.job_id)
    .bind(row.job_run_id)
    .fetch_all(&mut *tx)
    .await?;
    let task_statuses: Vec<String> = task_statuses.into_iter().map(|(status,)| status).collect();
    let result_refs = row.result_refs.as_ref().unwrap_or(&Value::Null);
    let canonical_state = canonical_work_run_state(state, result_refs, &task_statuses);
    let mut canonical_outcome =
        canonical_work_run_outcome(canonical_state, result_refs, &task_statuses);
    if !matches!(canonical_state, WorkRunState::Succeeded) {
        if let Some(summary) = finalize.result_summary.as_deref() {
            canonical_outcome["summary"] = Value::String(summary.to_string());
        }
    }
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "UPDATE bear_work_runs
         SET state = $2,
             result_summary = $3,
             result_refs = COALESCE(result_refs, '{{}}'::jsonb)
                 || jsonb_build_object('outcome', $4::jsonb)
         WHERE id = $1
         RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(run_id)
    .bind(canonical_state.as_str())
    .bind(canonical_outcome["summary"].as_str())
    .bind(&canonical_outcome)
    .fetch_one(&mut *tx)
    .await?;

    if matches!(canonical_state, WorkRunState::Succeeded) {
        settle_completed_job(&mut tx, &row, &canonical_outcome).await?;
    } else {
        settle_failed_work_as_blocked(&mut tx, &row, &canonical_outcome).await?;
    }

    tx.commit().await?;
    Ok(row)
}

/// Any non-success terminal work result stops the job by default. The failure
/// belongs to the work run and job: it must not be projected as a task-local
/// blocker for unfinished tasks.
async fn settle_failed_work_as_blocked(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    work_run: &WorkRunRow,
    outcome: &Value,
) -> Result<(), DenError> {
    let evidence = json!({
        "work_run_id": work_run.id,
        "state": work_run.state,
        "cancel_requested_by": work_run.cancel_requested_by,
        "cancel_reason": work_run.cancel_reason,
        "cancel_requested_at": work_run.cancel_requested_at,
    });
    let summary = work_run
        .cancel_reason
        .as_deref()
        .map(|reason| format!("Work run cancelled: {reason}"))
        .unwrap_or_else(|| {
            format!(
                "Work run ended {} before completion could be verified.",
                work_run.state
            )
        });

    let is_current = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM bear_jobs WHERE id = $1 AND current_run_id = $2
         )",
    )
    .bind(work_run.job_id)
    .bind(work_run.job_run_id)
    .fetch_one(&mut **tx)
    .await?;
    if !is_current {
        return Ok(());
    }

    sqlx::query(
        "UPDATE bear_job_runs SET state = 'blocked', outcome = $2::jsonb,
             finished_at = COALESCE(finished_at, NOW()), updated_at = NOW()
         WHERE id = $1",
    )
    .bind(work_run.job_run_id)
    .bind(outcome)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE bear_work_runs
         SET state = 'cancelled',
             result_summary = COALESCE(result_summary, $3),
             result_refs = COALESCE(result_refs, '{}'::jsonb) || $4::jsonb,
             finished_at = now(), updated_at = now()
         WHERE job_id = $1 AND job_run_id = $2 AND state = 'queued'",
    )
    .bind(work_run.job_id)
    .bind(work_run.job_run_id)
    .bind(&summary)
    .bind(&evidence)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO bear_job_events (job_id, run_id, event_type, by_role, payload)
         VALUES ($1, $2, 'job_blocked', 'system', $3::jsonb)",
    )
    .bind(work_run.job_id)
    .bind(work_run.job_run_id)
    .bind(json!({"status": "blocked", "source": "failed_work_run", "work_run": evidence}))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A work run is the terminal executor for its Docket run. Once it succeeds,
/// settle the owning job in the same transaction when every task and criterion
/// has reached its terminal success state. This avoids requiring a redundant
/// `execute_job` call solely to persist the already-established outcome.
async fn settle_completed_job(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    work_run: &WorkRunRow,
    outcome: &Value,
) -> Result<(), DenError> {
    let tasks_complete = sqlx::query_scalar::<_, bool>(
        "SELECT NOT EXISTS (
             SELECT 1
             FROM bear_tasks t
             LEFT JOIN bear_task_run_state s ON s.task_id = t.id AND s.run_id = $2
             WHERE t.job_id = $1 AND COALESCE(s.status, 'pending') NOT IN ('done', 'cancelled')
         )",
    )
    .bind(work_run.job_id)
    .bind(work_run.job_run_id)
    .fetch_one(&mut **tx)
    .await?;
    let criteria_complete = sqlx::query_scalar::<_, bool>(
        "SELECT NOT EXISTS (
             SELECT 1
             FROM bear_job_criteria c
             LEFT JOIN bear_job_criteria_state s ON s.criterion_id = c.id AND s.run_id = $2
             WHERE c.job_id = $1 AND COALESCE(s.status, 'unmet') NOT IN ('met', 'waived')
         )",
    )
    .bind(work_run.job_id)
    .bind(work_run.job_run_id)
    .fetch_one(&mut **tx)
    .await?;
    if !tasks_complete || !criteria_complete {
        return Ok(());
    }

    let is_current = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM bear_jobs WHERE id = $1 AND current_run_id = $2
         )",
    )
    .bind(work_run.job_id)
    .bind(work_run.job_run_id)
    .fetch_one(&mut **tx)
    .await?;
    if !is_current {
        return Ok(());
    }

    sqlx::query(
        "UPDATE bear_job_runs
         SET state = 'completed', outcome = $2::jsonb,
             finished_at = COALESCE(finished_at, NOW()), updated_at = NOW()
         WHERE id = $1",
    )
    .bind(work_run.job_run_id)
    .bind(outcome)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO bear_job_events (job_id, run_id, event_type, by_role, payload)
         VALUES ($1, $2, 'job_completed', 'system', $3::jsonb)",
    )
    .bind(work_run.job_id)
    .bind(work_run.job_run_id)
    .bind(json!({"status": "completed", "source": "work_run"}))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod outcome_tests {
    use super::*;

    #[test]
    fn cargo_cache_miss_blocks_even_when_turn_completed() {
        let refs = json!({
            "cargo_failure": {
                "code": "cargo_offline_cache_miss",
                "required_package": "serde"
            },
            "turn_outcome": { "kind": "completed" },
            "armature_report": { "status_hint": "completed" }
        });
        let task_statuses = vec!["pending".to_string()];
        let state = canonical_work_run_state(WorkRunState::Succeeded, &refs, &task_statuses);
        let outcome = canonical_work_run_outcome(state, &refs, &task_statuses);
        assert_eq!(outcome["status"], "blocked");
        assert_eq!(outcome["code"], "cargo_offline_cache_miss");
        assert!(outcome["summary"].as_str().unwrap().contains("serde"));
        assert!(outcome["next_action"]
            .as_str()
            .unwrap()
            .contains("Prepare Rust dependencies"));
    }

    #[test]
    fn unfinished_tasks_prevent_completed_outcome() {
        let refs = json!({ "turn_outcome": { "kind": "completed" } });
        let task_statuses = vec!["done".to_string(), "pending".to_string()];
        let state = canonical_work_run_state(WorkRunState::Succeeded, &refs, &task_statuses);
        let outcome = canonical_work_run_outcome(state, &refs, &task_statuses);
        assert_eq!(state, WorkRunState::Blocked);
        assert_eq!(outcome["status"], "incomplete");
        assert_eq!(outcome["code"], "work_incomplete");
    }

    #[test]
    fn terminal_worker_failures_remain_authoritative() {
        let refs = json!({
            "cargo_failure": { "code": "cargo_offline_cache_miss" },
            "turn_outcome": { "kind": "completed" },
        });
        let task_statuses = vec!["pending".to_string()];
        let state = canonical_work_run_state(WorkRunState::TimedOut, &refs, &task_statuses);
        let outcome = canonical_work_run_outcome(state, &refs, &task_statuses);
        assert_eq!(state, WorkRunState::TimedOut);
        assert_eq!(outcome["status"], "timed_out");
        assert_eq!(outcome["code"], "work_timed_out");
    }

    #[test]
    fn blocked_task_beats_unfinished_task_count() {
        let refs = json!({ "turn_outcome": { "kind": "completed" } });
        let task_statuses = vec!["blocked".to_string(), "pending".to_string()];
        let state = canonical_work_run_state(WorkRunState::Succeeded, &refs, &task_statuses);
        let outcome = canonical_work_run_outcome(state, &refs, &task_statuses);
        assert_eq!(state, WorkRunState::Blocked);
        assert_eq!(outcome["status"], "blocked");
        assert_eq!(outcome["code"], "task_blocked");
    }
}

/// Ask the owning worker to cancel; teardown happens asynchronously. Returns
/// false when the run is already terminal.
pub async fn request_work_run_cancel(
    pool: &PgPool,
    run_id: Uuid,
    bear_id: Uuid,
) -> Result<bool, DenError> {
    request_work_run_cancel_with_provenance(
        pool,
        run_id,
        bear_id,
        &WorkRunCancelRequest {
            requested_by: "system".into(),
            reason: "cancellation requested without caller provenance".into(),
        },
    )
    .await
}

/// Ask the owning worker to cancel with caller provenance; teardown happens
/// asynchronously. Returns false when the run is already terminal.
pub async fn request_work_run_cancel_with_provenance(
    pool: &PgPool,
    run_id: Uuid,
    bear_id: Uuid,
    request: &WorkRunCancelRequest,
) -> Result<bool, DenError> {
    let result = sqlx::query(
        "UPDATE bear_work_runs
         SET cancel_requested = TRUE,
             cancel_requested_by = COALESCE(cancel_requested_by, $3),
             cancel_reason = COALESCE(cancel_reason, $4),
             cancel_requested_at = COALESCE(cancel_requested_at, now()),
             updated_at = now()
         WHERE id = $1 AND bear_id = $2
           AND state IN ('queued', 'claimed', 'provisioning', 'running', 'reporting')",
    )
    .bind(run_id)
    .bind(bear_id)
    .bind(&request.requested_by)
    .bind(&request.reason)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The latest work run bound to a BearWire session, including terminal runs.
pub async fn get_work_run_by_session(
    pool: &PgPool,
    session_id: &str,
) -> Result<Option<WorkRunRow>, DenError> {
    // sqlx-dynamic: the shared typed work-run column projection is assembled centrally.
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "SELECT {WORK_RUN_COLUMNS} FROM bear_work_runs
         WHERE bearwire_session_id = $1
         ORDER BY updated_at DESC
         LIMIT 1"
    ))
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
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

/// Project the native BearWire permission obligation onto an attached work
/// run. The obligation remains the authority; this state is diagnostic only
/// and cannot grant permission.
pub async fn mark_attached_work_run_permission_required(
    pool: &PgPool,
    session_id: &str,
) -> Result<bool, DenError> {
    // ponytail: runtime SQL until Phase 4 migration metadata can be prepared against Postgres;
    // upgrade to query! when cargo-sqlx and a migrated database are available.
    // sqlx-dynamic: checked metadata cannot be generated in this database-free session.
    let result = sqlx::query(
        "UPDATE bear_work_runs
         SET attachment_state = 'permission_required', updated_at = now()
         WHERE attached_client_session_id = $1
           AND execution_target = 'attached_armature'
           AND state IN ('queued', 'claimed', 'provisioning', 'running', 'paused', 'reporting')
           AND attachment_state IN ('attached', 'permission_required')",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Clear the diagnostic permission state after the authoritative obligation
/// accepts a current client decision. Late and conflicting decisions never
/// reach this function.
pub async fn settle_attached_work_run_permission(
    pool: &PgPool,
    session_id: &str,
) -> Result<bool, DenError> {
    // ponytail: runtime SQL until Phase 4 migration metadata can be prepared against Postgres;
    // upgrade to query! when cargo-sqlx and a migrated database are available.
    // sqlx-dynamic: checked metadata cannot be generated in this database-free session.
    let result = sqlx::query(
        "UPDATE bear_work_runs
         SET attachment_state = 'attached', updated_at = now()
         WHERE attached_client_session_id = $1
           AND execution_target = 'attached_armature'
           AND state IN ('queued', 'claimed', 'provisioning', 'running', 'paused', 'reporting')
           AND attachment_state = 'permission_required'",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn disconnect_attached_work_run(
    pool: &PgPool,
    session_id: &str,
    timeout: StdDuration,
) -> Result<Option<WorkRunRow>, DenError> {
    let deadline = OffsetDateTime::now_utc()
        + time::Duration::try_from(timeout)
            .map_err(|_| DenError::ValidationError("disconnect timeout is too large".into()))?;
    // sqlx-dynamic: the shared WORK_RUN_COLUMNS projection keeps WorkRunRow decoding aligned.
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "UPDATE bear_work_runs
         SET state = 'paused', attachment_state = 'disconnected',
             disconnected_at = COALESCE(disconnected_at, now()),
             disconnect_deadline_at = COALESCE(disconnect_deadline_at, $2),
             runner_id = NULL, lease_expires_at = NULL, updated_at = now()
         WHERE attached_client_session_id = $1
           AND execution_target = 'attached_armature'
           AND state IN ('queued', 'claimed', 'provisioning', 'running', 'paused', 'reporting')
           AND attachment_state <> 'timed_out'
         RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(session_id)
    .bind(deadline)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn reconnect_attached_work_run(
    pool: &PgPool,
    session_id: &str,
) -> Result<Option<WorkRunRow>, DenError> {
    // sqlx-dynamic: the shared WORK_RUN_COLUMNS projection keeps WorkRunRow decoding aligned.
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "UPDATE bear_work_runs
         SET attachment_state = 'attached', disconnected_at = NULL,
             disconnect_deadline_at = NULL, updated_at = now()
         WHERE attached_client_session_id = $1
           AND execution_target = 'attached_armature'
           AND state = 'paused' AND attachment_state = 'disconnected'
           AND disconnect_deadline_at > now()
         RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn timeout_disconnected_work_runs(pool: &PgPool) -> Result<Vec<WorkRunRow>, DenError> {
    // sqlx-dynamic: the shared WORK_RUN_COLUMNS projection keeps WorkRunRow decoding aligned.
    let rows = sqlx::query_as::<_, WorkRunRow>(&format!(
        "UPDATE bear_work_runs
         SET state = 'timed_out', attachment_state = 'timed_out',
             result_summary = 'Attached armature disconnected and did not reconnect before the deadline.',
             result_refs = COALESCE(result_refs, '{{}}'::jsonb) || jsonb_build_object(
                 'outcome', jsonb_build_object(
                     'status', 'timed_out',
                     'code', 'armature_disconnect_timeout',
                     'summary', 'Attached armature disconnected and did not reconnect before the deadline.',
                     'next_action', 'Reconnect the armature and recover this run.'
                 ),
                 'attachment', jsonb_build_object(
                     'disconnected_at', disconnected_at,
                     'deadline_at', disconnect_deadline_at
                 )
             ),
             error = 'armature_disconnect_timeout', finished_at = COALESCE(finished_at, now()),
             runner_id = NULL, lease_expires_at = NULL, updated_at = now()
         WHERE execution_target = 'attached_armature'
           AND state = 'paused' AND attachment_state = 'disconnected'
           AND disconnect_deadline_at <= now()
         RETURNING {WORK_RUN_COLUMNS}"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn is_attached_recovery_source(
    execution_target: &str,
    state: &str,
    result_refs: Option<&Value>,
) -> bool {
    execution_target == "attached_armature"
        && state == "timed_out"
        && result_refs
            .and_then(|refs| refs.pointer("/outcome/code"))
            .and_then(Value::as_str)
            == Some("armature_disconnect_timeout")
}

pub async fn recover_attached_work_run(
    pool: &PgPool,
    source_run_id: Uuid,
    bear_id: Uuid,
) -> Result<WorkRunRow, DenError> {
    let mut tx = pool.begin().await?;
    // sqlx-dynamic: the shared WORK_RUN_COLUMNS projection keeps WorkRunRow decoding aligned.
    let source = sqlx::query_as::<_, WorkRunRow>(&format!(
        "SELECT {WORK_RUN_COLUMNS} FROM bear_work_runs
         WHERE id = $1 AND bear_id = $2 FOR UPDATE"
    ))
    .bind(source_run_id)
    .bind(bear_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DenError::NotFound(format!("work run not found: {source_run_id}")))?;
    if !is_attached_recovery_source(
        &source.execution_target,
        &source.state,
        source.result_refs.as_ref(),
    ) {
        return Err(DenError::ValidationError(
            "work run is not eligible for attached-armature recovery".into(),
        ));
    }
    let session_id = source
        .attached_client_session_id
        .as_deref()
        .ok_or_else(|| {
            DenError::ValidationError("recovery source has no attached session".into())
        })?;
    // ponytail: runtime SQL until Phase 4 migration metadata can be prepared against Postgres;
    // upgrade to query_scalar! when cargo-sqlx and a migrated database are available.
    // sqlx-dynamic: checked metadata cannot be generated in this database-free session.
    let attempt: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(attempt), 0) + 1 FROM bear_work_runs WHERE job_id = $1",
    )
    .bind(source.job_id)
    .fetch_one(&mut *tx)
    .await?;
    // sqlx-dynamic: the shared WORK_RUN_COLUMNS projection keeps WorkRunRow decoding aligned.
    let recovered = sqlx::query_as::<_, WorkRunRow>(&format!(
        "INSERT INTO bear_work_runs (
             bear_id, job_id, job_run_id, attempt, root_name, git_ref, image_name,
             execution_target, attached_client_session_id, attachment_state,
             attachment_warning, result_refs
         ) VALUES (
             $1, $2, $3, $4, $5, $6, $7,
             'attached_armature', $8, 'attached', $9,
             jsonb_build_object('recovery', jsonb_build_object(
                 'source_work_run_id', $10::text,
                 'source_outcome', COALESCE($11::jsonb, '{{}}'::jsonb)
             ))
         ) RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(source.bear_id)
    .bind(source.job_id)
    .bind(source.job_run_id)
    .bind(attempt)
    .bind(source.root_name.as_deref())
    .bind(source.git_ref.as_deref())
    .bind(source.image_name.as_deref())
    .bind(session_id)
    .bind(source.attachment_warning.as_deref())
    .bind(source.id)
    .bind(source.result_refs.as_ref())
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| match err {
        sqlx::Error::Database(db)
            if db.constraint() == Some("idx_bear_work_runs_one_active_per_job") =>
        {
            DenError::ValidationError(
                "recovery already started or the job has another active work run".into(),
            )
        }
        other => other.into(),
    })?;
    tx.commit().await?;
    Ok(recovered)
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

    type ActiveTaskRow = (
        Uuid,
        String,
        String,
        sqlx::types::Json<Vec<String>>,
        Option<String>,
    );
    let active_task: Option<ActiveTaskRow> = sqlx::query_as(
        "SELECT t.id, t.title, t.body, t.completion_criteria, t.difficulty
         FROM bear_tasks t
         JOIN bear_task_run_state s ON s.task_id = t.id AND s.run_id = $2
         WHERE t.job_id = $1 AND s.status = 'in_progress'
         ORDER BY t.sibling_order, t.created_at
         LIMIT 1",
    )
    .bind(run.job_id)
    .bind(run.job_run_id)
    .fetch_optional(pool)
    .await?;
    let task = match active_task {
        Some(task) => task,
        None => {
            let service = PgDocketService::from_pool(pool);
            let task = service
                .runnable_work_tasks(run.bear_id, 500)
                .await?
                .into_iter()
                .find(|task| task.task.job_id == Some(run.job_id))
                .ok_or_else(|| {
                    DenError::ValidationError("job has no runnable work task for checkout".into())
                })?
                .task;
            (
                task.id,
                task.title,
                task.body,
                task.completion_criteria,
                task.difficulty
                    .map(|difficulty| difficulty.as_str().to_string()),
            )
        }
    };
    // A checkout assigns one durable task to this work run. Persist that
    // assignment before starting the turn so cancellation can block exactly
    // this task rather than guessing from the job's pending tasks.
    sqlx::query(
        "UPDATE bear_task_run_state
         SET status = 'pending', updated_at = NOW()
         WHERE run_id = $1 AND task_id <> $2 AND status = 'in_progress'",
    )
    .bind(run.job_run_id)
    .bind(task.0)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO bear_task_run_state (
             run_id, task_id, status, started_at, updated_at
         ) VALUES ($1, $2, 'in_progress', NOW(), NOW())
         ON CONFLICT (run_id, task_id) DO UPDATE
         SET status = 'in_progress',
             started_at = COALESCE(bear_task_run_state.started_at, NOW()),
             finished_at = NULL,
             updated_at = NOW()",
    )
    .bind(run.job_run_id)
    .bind(task.0)
    .execute(pool)
    .await?;

    let difficulty = task.4.as_deref().and_then(parse_task_difficulty);
    let resolved_profile = resolve_execution_profile(difficulty);
    let persisted_profile = resolved_profile.persisted_value();
    let routing = route_turn(
        pool,
        TurnIntent {
            // One work run is one durable dispatch turn. Re-checkout reuses it.
            idempotency_key: run.id,
            bear_id,
            job_id: run.job_id,
            run_id: run.job_run_id,
            task_id: task.0,
            source: TurnSource::Dispatch,
            originating_conversation_id: None,
            parent_conversation_id: None,
            surface: if run.execution_target == "attached_armature" {
                ExecutionSurface::Armature
            } else {
                ExecutionSurface::Sandbox
            },
            resolved_profile: Some(persisted_profile),
            attempt: run.attempt,
        },
    )
    .await?;
    start_turn_attempt(
        pool,
        routing.id,
        Some(run.id),
        run.attempt,
        resolved_profile,
    )
    .await?;
    let tasks = vec![(task.0, task.1, task.2, task.3)];
    let (goal, commit_policy): (String, Option<String>) =
        sqlx::query_as("SELECT goal, commit_policy FROM bear_jobs WHERE id = $1")
            .bind(run.job_id)
            .fetch_one(pool)
            .await?;
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
            task_id: Some(task.0),
            state: "active".to_string(),
        },
    )
    .await?;

    let task_title = format!(
        "{} work task{}",
        tasks.len(),
        if tasks.len() == 1 { "" } else { "s" }
    );
    let prompt = build_work_prompt(
        run.job_id,
        run.job_run_id,
        &goal,
        &tasks,
        commit_policy.as_deref(),
    );
    Ok(WorkRunCheckout {
        run,
        prompt,
        task_title,
    })
}

fn parse_task_difficulty(raw: &str) -> Option<DocketTaskDifficulty> {
    Some(match raw {
        "trivial" => DocketTaskDifficulty::Trivial,
        "moderate" => DocketTaskDifficulty::Moderate,
        "hard" => DocketTaskDifficulty::Hard,
        "unknown" => DocketTaskDifficulty::Unknown,
        _ => return None,
    })
}

fn build_work_prompt(
    job_id: Uuid,
    run_id: Uuid,
    goal: &str,
    tasks: &[(Uuid, String, String, sqlx::types::Json<Vec<String>>)],
    commit_policy: Option<&str>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are executing a Docket task autonomously in the work stance, inside a sandbox.\n\n",
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
    match commit_policy {
        Some("per_task") => prompt.push_str(
            "- Commit the completed task with a clear, specific git commit message; Den publishes that commit to the job's work branch before the next task runs.\n\
             - Do not push, deploy, or call external services; publishing happens outside the sandbox.\n",
        ),
        Some("per_job") => prompt.push_str(
            "- Commit your work as you go with clear, specific git commit messages; Den publishes the final job commit to the job's work branch after the job completes.\n\
             - Do not push, deploy, or call external services; publishing happens outside the sandbox after the job completes.\n",
        ),
        _ => prompt.push_str("- Do not push, publish, deploy, or call external services.\n"),
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
           AND ($3::text IS NULL OR state = $3)
         ORDER BY queued_at DESC
         LIMIT $4"
    ))
    .bind(filter.bear_id)
    .bind(filter.job_id)
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

/// Run-scoped states for every unfinished task in a job. A job-scoped work run
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
         WHERE t.job_id = $1
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

/// Bears that have jobs with unfinished tasks, for the optional auto-enqueue
/// sweep (`WORK_DISPATCH_AUTO`).
pub async fn list_bears_with_work_tasks(pool: &PgPool) -> Result<Vec<Uuid>, DenError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT DISTINCT bear_id FROM bear_jobs WHERE status IN ('ready', 'running', 'blocked')",
    )
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
    pub work_surface_name: Option<String>,
    pub commit_policy: Option<String>,
    pub work_branch: Option<String>,
    pub allow_default_ref: bool,
    /// Validated child summaries for this job run. Raw child transcripts and tool traces
    /// are intentionally not projected into the dispatch context.
    pub child_result_rollups: serde_json::Value,
}

impl WorkRunDispatchContext {
    /// Whether the job's commit policy publishes successful runs to the
    /// upstream work branch (`per_task` / `per_job`).
    pub fn publishes(&self) -> bool {
        matches!(self.commit_policy.as_deref(), Some("per_task" | "per_job"))
    }
}

pub async fn get_work_run_dispatch_context(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<WorkRunDispatchContext, DenError> {
    sqlx::query_as::<_, WorkRunDispatchContext>(
        "SELECT b.slug AS bear_slug, b.name AS bear_name, j.created_by_user_id, j.goal AS job_goal, s.name AS work_surface_name,
                j.commit_policy, j.work_branch,
                COALESCE(j.work_branch = s.default_ref, FALSE) AS allow_default_ref,
                COALESCE((
                    SELECT jsonb_agg(
                        jsonb_build_object('summary', rr.summary, 'evidence_refs', rr.evidence_refs)
                        ORDER BY rr.created_at, rr.task_id
                    )
                    FROM docket_result_rollups rr
                    WHERE rr.run_id = r.job_run_id
                      AND rr.parent_task_id = (
                          SELECT task_state.task_id
                          FROM bear_task_run_state task_state
                          WHERE task_state.run_id = r.job_run_id
                            AND task_state.status = 'in_progress'
                          LIMIT 1
                      )
                ), '[]'::jsonb) AS child_result_rollups
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{effective_work_run_root, is_attached_recovery_source};

    #[test]
    fn attached_recovery_requires_the_canonical_timeout_outcome() {
        let timeout = json!({"outcome": {"code": "armature_disconnect_timeout"}});
        assert!(is_attached_recovery_source(
            "attached_armature",
            "timed_out",
            Some(&timeout)
        ));
        assert!(!is_attached_recovery_source(
            "sandbox",
            "timed_out",
            Some(&timeout)
        ));
        assert!(!is_attached_recovery_source(
            "attached_armature",
            "failed",
            Some(&timeout)
        ));
        assert!(!is_attached_recovery_source(
            "attached_armature",
            "timed_out",
            Some(&json!({"outcome": {"code": "activity_timeout"}}))
        ));
    }

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
