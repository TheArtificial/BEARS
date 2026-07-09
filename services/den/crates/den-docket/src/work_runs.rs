//! Durable dispatch/claim/lease state for autonomous `work`-stance execution.
//!
//! One `bear_work_runs` row per dispatch attempt of one task. The dispatch
//! worker (den-runtime) claims rows with a lease; the BearWire `work.*`
//! methods bind the in-sandbox armature's session and record the turn
//! outcome; the worker harvests and finalizes. Lifecycle audit reuses
//! `bear_task_events` (Docket schedules, gates, and records — it never
//! executes task bodies; ADR-0034).

use serde_json::{json, Value};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

use crate::model::DocketExecutionSessionUpsert;

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

const WORK_RUN_COLUMNS: &str = "id, bear_id, job_id, task_id, job_run_id, attempt, state, \
     runner_id, lease_expires_at, cancel_requested, root_name, git_ref, image_name, \
     sandbox_server_url, sandbox_id, sandbox_type, sandbox_strength, work_surface, \
     bearwire_session_id, result_summary, result_refs, usage, error, \
     queued_at, started_at, finished_at, updated_at";

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct WorkRunRow {
    pub id: Uuid,
    pub bear_id: Uuid,
    pub job_id: Uuid,
    pub task_id: Uuid,
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

#[derive(Clone, Debug)]
pub struct WorkRunEnqueue {
    pub bear_id: Uuid,
    pub task_id: Uuid,
    pub root_name: Option<String>,
    pub git_ref: Option<String>,
    /// Catalog image name on the sandbox provider (None = root/provider default).
    pub image_name: Option<String>,
    /// Recorded on the audit event; also the identity work-run tokens are
    /// minted under (v1: only user-created jobs dispatch to work).
    pub requested_by_user_id: Option<i32>,
}

/// Queue a work run for a task assigned to the `work` stance. Ensures the
/// job has an active `bear_job_runs` row (creating an `event`-triggered one
/// when needed). The partial unique index rejects a second active run for the
/// same task.
pub async fn enqueue_work_run(
    pool: &PgPool,
    enqueue: WorkRunEnqueue,
) -> Result<WorkRunRow, DenError> {
    let mut tx = pool.begin().await?;

    let task: Option<(Uuid, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT bear_id, job_id, assigned_to_role FROM bear_tasks WHERE id = $1 AND bear_id = $2",
    )
    .bind(enqueue.task_id)
    .bind(enqueue.bear_id)
    .fetch_optional(&mut *tx)
    .await?
    .map(|row: (Uuid, Option<Uuid>, Option<String>)| row);

    let Some((_, job_id, assigned_to_role)) = task else {
        return Err(DenError::NotFound(format!(
            "Docket task not found: {}",
            enqueue.task_id
        )));
    };
    let Some(job_id) = job_id else {
        return Err(DenError::ValidationError(
            "task is not part of a Docket job; only job tasks can dispatch to work".into(),
        ));
    };
    if assigned_to_role.as_deref() != Some("work") {
        return Err(DenError::ValidationError(format!(
            "task {} is not assigned to the work stance (assigned_to_role={})",
            enqueue.task_id,
            assigned_to_role.as_deref().unwrap_or("<none>")
        )));
    }

    // Reuse the job's current run when it is still live; otherwise open a new
    // event-triggered run so run-scoped task state has a home.
    let current_run: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT r.id, r.state FROM bear_jobs j
         JOIN bear_job_runs r ON r.id = j.current_run_id
         WHERE j.id = $1",
    )
    .bind(job_id)
    .fetch_optional(&mut *tx)
    .await?;
    let job_run_id = match current_run {
        Some((run_id, state)) if !matches!(state.as_str(), "completed" | "failed" | "cancelled") => {
            run_id
        }
        _ => {
            let (run_id,): (Uuid,) = sqlx::query_as(
                "INSERT INTO bear_job_runs (job_id, trigger, state)
                 VALUES ($1, 'event', 'running')
                 RETURNING id",
            )
            .bind(job_id)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query("UPDATE bear_jobs SET current_run_id = $2, updated_at = now() WHERE id = $1")
                .bind(job_id)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            run_id
        }
    };

    let (attempt,): (i32,) = sqlx::query_as(
        "SELECT COALESCE(MAX(attempt), 0) + 1 FROM bear_work_runs WHERE task_id = $1",
    )
    .bind(enqueue.task_id)
    .fetch_one(&mut *tx)
    .await?;

    let inserted = sqlx::query_as::<_, WorkRunRow>(&format!(
        "INSERT INTO bear_work_runs (bear_id, job_id, task_id, job_run_id, attempt, root_name, git_ref, image_name)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING {WORK_RUN_COLUMNS}"
    ))
    .bind(enqueue.bear_id)
    .bind(job_id)
    .bind(enqueue.task_id)
    .bind(job_run_id)
    .bind(attempt)
    .bind(enqueue.root_name.as_deref())
    .bind(enqueue.git_ref.as_deref())
    .bind(enqueue.image_name.as_deref())
    .fetch_one(&mut *tx)
    .await;

    let run = match inserted {
        Ok(run) => run,
        Err(sqlx::Error::Database(db)) if db.constraint() == Some("idx_bear_work_runs_one_active_per_task") => {
            return Err(DenError::ValidationError(format!(
                "task {} already has an active work run",
                enqueue.task_id
            )));
        }
        Err(e) => return Err(e.into()),
    };

    append_task_event(
        &mut tx,
        run.task_id,
        run.job_run_id,
        "claimed",
        enqueue.requested_by_user_id,
        json!({ "work_run_id": run.id, "attempt": run.attempt, "phase": "enqueued" }),
    )
    .await?;

    tx.commit().await?;
    Ok(run)
}

/// Claim the next dispatchable run with a lease (`FOR UPDATE SKIP LOCKED`).
/// Picks up fresh `queued` runs and takes over non-terminal runs whose lease
/// expired (worker crash); the state of a taken-over run is preserved so the
/// new owner can reconcile rather than restart blindly.
pub async fn claim_next_work_run(
    pool: &PgPool,
    runner_id: &str,
    lease: std::time::Duration,
) -> Result<Option<WorkRunRow>, DenError> {
    let lease_secs = i64::try_from(lease.as_secs()).unwrap_or(i64::MAX);
    let row = sqlx::query_as::<_, WorkRunRow>(&format!(
        "WITH candidate AS (
             SELECT id FROM bear_work_runs
             WHERE state = 'queued'
                OR (state IN ('claimed', 'provisioning', 'running', 'reporting')
                    AND lease_expires_at IS NOT NULL AND lease_expires_at < now())
             ORDER BY queued_at ASC
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
    append_task_event(
        &mut tx,
        row.task_id,
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

    tx.commit().await?;
    Ok(row)
}

/// Ask the owning worker to cancel; teardown happens asynchronously. Returns
/// false when the run is already terminal.
pub async fn request_work_run_cancel(pool: &PgPool, run_id: Uuid, bear_id: Uuid) -> Result<bool, DenError> {
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
/// the Docket execution session (owner_profile = work, which satisfies the
/// Work-stance gate), and build the non-interactive prompt from the durable
/// task definition.
pub async fn checkout_work_run_for_session(
    pool: &PgPool,
    run_id: Uuid,
    bear_id: Uuid,
    session_id: &str,
) -> Result<WorkRunCheckout, DenError> {
    let run = bind_work_run_session(pool, run_id, bear_id, session_id).await?;

    let task: (String, String, sqlx::types::Json<Vec<String>>) = sqlx::query_as(
        "SELECT title, body, completion_criteria FROM bear_tasks WHERE id = $1",
    )
    .bind(run.task_id)
    .fetch_one(pool)
    .await?;
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
            task_id: Some(run.task_id),
            state: "active".to_string(),
        },
    )
    .await?;

    let (task_title, task_body, criteria) = (task.0, task.1, task.2 .0);
    let prompt = build_work_prompt(&goal, &task_title, &task_body, &criteria, publishes);
    Ok(WorkRunCheckout {
        run,
        prompt,
        task_title,
    })
}

fn build_work_prompt(
    goal: &str,
    title: &str,
    body: &str,
    criteria: &[String],
    publishes: bool,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are executing a Docket task autonomously in the work stance, inside a sandbox. \
         No user is present in this session and none will respond — never wait for user input \
         or ask questions.\n\n",
    );
    prompt.push_str(&format!("Job objective: {goal}\n\n"));
    prompt.push_str(&format!("Task: {title}\n"));
    if !body.trim().is_empty() {
        prompt.push_str(&format!("{body}\n"));
    }
    prompt.push_str("\nCompletion criteria — the task is done only when all of these hold:\n");
    for criterion in criteria {
        prompt.push_str(&format!("- {criterion}\n"));
    }
    prompt.push_str(
        "\nRules:\n\
         - Operate only inside the sandbox workspace; it contains the work surface.\n\
         - When every criterion is satisfied, mark the task done using the \
           update_current_task_status tool — this is how success is recorded.\n\
         - If you cannot make progress, mark the task blocked with a specific reason \
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
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM bear_task_run_state WHERE run_id = $1 AND task_id = $2",
    )
    .bind(job_run_id)
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(status,)| status))
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
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT DISTINCT bear_id FROM bear_tasks WHERE assigned_to_role = 'work'",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Everything the dispatch worker needs to provision a claimed run.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct WorkRunDispatchContext {
    pub bear_slug: String,
    pub created_by_user_id: i32,
    pub job_goal: String,
    pub work_surface_ref: Option<String>,
    pub commit_policy: Option<String>,
    pub work_branch: Option<String>,
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
        "SELECT b.slug AS bear_slug, j.created_by_user_id, j.goal AS job_goal, j.work_surface_ref,
                j.commit_policy, j.work_branch
         FROM bear_work_runs r
         JOIN bears b ON b.id = r.bear_id
         JOIN bear_jobs j ON j.id = r.job_id
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
