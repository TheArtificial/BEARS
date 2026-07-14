//! Docket Postgres access — internal to the module.
//!
//! These functions are the persistence layer behind `DocketService`; callers
//! outside Docket go through the service, never here. Docket job/task data is
//! stored in the ADR-0034 relational tables.

use std::collections::HashMap;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

use super::model::{
    docket_parent_task_ref, docket_task_status_from_task_list_item_status,
    normalize_completion_criteria, task_list_projection_from_docket_job,
    validate_docket_job_create, validate_docket_task_create, DocketCriterionStateRow,
    DocketCriterionStateUpdate, DocketExecutionLookup, DocketExecutionSessionRow,
    DocketExecutionSessionUpsert, DocketJobCreate, DocketJobCriterionRow, DocketJobExecuteOutcome,
    DocketJobExecuteRequest, DocketJobListFilter, DocketJobProjection, DocketJobRow,
    DocketJobRunRow, DocketJobStatus, DocketJobUpdate, DocketTaskCreate, DocketTaskDefinitionPatch,
    DocketTaskInput, DocketTaskListFilter, DocketTaskProjection, DocketTaskRow,
    DocketTaskRunStateRow, DocketTaskUpdate, TaskListItemStatus, TaskListProjection,
    TaskListSourceRef, TaskListSyncOutcome, TaskListSyncRequest, TaskListSyncState,
};

pub(super) async fn create_job(
    pool: &PgPool,
    create: DocketJobCreate,
) -> Result<DocketJobProjection, DenError> {
    validate_docket_job_create(&create)?;

    let mut tx = pool.begin().await?;
    let job = sqlx::query_as::<_, DocketJobRow>(
        r"
        INSERT INTO bear_jobs (
            bear_id, created_by_user_id, created_by_role, goal, work_surface_ref, work_surface_id,
            commit_policy, work_branch, status, visibility
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref, work_surface_id,
                  commit_policy, work_branch, status, visibility, current_run_id, created_at, updated_at
        ",
    )
    .bind(create.bear_id)
    .bind(create.created_by_user_id)
    .bind(create.created_by_role.trim())
    .bind(create.goal.trim())
    .bind(create.work_surface_ref.as_deref())
    .bind(create.work_surface_id)
    .bind(create.commit_policy.map(|policy| policy.as_str()))
    .bind(
        create
            .work_branch
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty()),
    )
    .bind(create.status.as_str())
    .bind(create.visibility.as_str())
    .fetch_one(&mut *tx)
    .await?;

    let run = sqlx::query_as::<_, DocketJobRunRow>(
        r"
        INSERT INTO bear_job_runs (job_id, trigger, state)
        VALUES ($1, 'manual', 'dispatched')
        RETURNING id, job_id, trigger, schedule_ref, state, started_at, finished_at,
                  outcome, created_at, updated_at
        ",
    )
    .bind(job.id)
    .fetch_one(&mut *tx)
    .await?;

    let job = sqlx::query_as::<_, DocketJobRow>(
        r"
        UPDATE bear_jobs
        SET current_run_id = $2, updated_at = NOW()
        WHERE id = $1
        RETURNING id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref, work_surface_id,
                  commit_policy, work_branch, status, visibility, current_run_id, created_at, updated_at
        ",
    )
    .bind(job.id)
    .bind(run.id)
    .fetch_one(&mut *tx)
    .await?;

    let mut criteria = Vec::new();
    for criterion in &create.criteria {
        let row = sqlx::query_as::<_, DocketJobCriterionRow>(
            r"
            INSERT INTO bear_job_criteria (job_id, kind, description, spec, sibling_order)
            VALUES ($1, $2, $3, $4::jsonb, $5)
            RETURNING id, job_id, kind, description, spec, sibling_order, created_at, updated_at
            ",
        )
        .bind(job.id)
        .bind(criterion.kind.as_str())
        .bind(criterion.description.trim())
        .bind(criterion.spec.as_ref())
        .bind(criterion.sibling_order)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            r"
            INSERT INTO bear_job_criteria_state (run_id, criterion_id, status)
            VALUES ($1, $2, 'unmet')
            ",
        )
        .bind(run.id)
        .bind(row.id)
        .execute(&mut *tx)
        .await?;
        criteria.push(row);
    }

    let mut task_ids_by_client_key = HashMap::new();
    let mut tasks = Vec::new();
    for task in &create.tasks {
        let parent_task_id = resolve_parent_task_id(task, &task_ids_by_client_key)?;
        let row = insert_task_for_job(&mut tx, &create, job.id, &run, task, parent_task_id).await?;
        if let Some(key) = task
            .client_key
            .as_ref()
            .map(|key| key.trim())
            .filter(|key| !key.is_empty())
        {
            task_ids_by_client_key.insert(key.to_string(), row.id);
        }
        tasks.push(row);
    }

    sqlx::query(
        r"
        INSERT INTO bear_job_events (job_id, run_id, event_type, by_role, by_user_id, payload)
        VALUES ($1, $2, 'job_created', $3, $4, $5::jsonb)
        ",
    )
    .bind(job.id)
    .bind(run.id)
    .bind(create.created_by_role.trim())
    .bind(create.created_by_user_id)
    .bind(json!({
        "criteria_count": criteria.len(),
        "task_count": tasks.len(),
        "status": job.status,
        "visibility": job.visibility,
    }))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    let task_states = list_task_run_states(pool, run.id).await?;
    let criteria_states = list_criterion_states(pool, run.id).await?;
    Ok(DocketJobProjection {
        job,
        current_run: Some(run),
        criteria,
        criteria_states,
        tasks,
        task_states,
    })
}

fn docket_task_definition_payload(task: &DocketTaskRow) -> Value {
    json!({
        "task_id": task.id,
        "job_id": task.job_id,
        "parent_task_id": task.parent_task_id,
        "sibling_order": task.sibling_order,
        "kind": task.kind,
        "scope": task.scope,
        "title": task.title,
        "body": task.body,
        "completion_criteria": task.completion_criteria.0,
        "difficulty": task.difficulty,
        "effort_hint": task.effort_hint,
        "assigned_to_role": task.assigned_to_role,
    })
}

fn resolve_parent_task_id(
    task: &DocketTaskInput,
    task_ids_by_client_key: &HashMap<String, Uuid>,
) -> Result<Option<Uuid>, DenError> {
    if let Some(parent_task_id) = task.parent_task_id {
        return Ok(Some(parent_task_id));
    }
    if let Some(parent_key) = task
        .parent_client_key
        .as_ref()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
    {
        return task_ids_by_client_key
            .get(parent_key)
            .copied()
            .map(Some)
            .ok_or_else(|| {
                DenError::ValidationError(format!(
                    "Docket task parent_client_key `{parent_key}` must refer to an earlier task"
                ))
            });
    }
    Ok(None)
}

async fn insert_task_for_job(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    create: &DocketJobCreate,
    job_id: Uuid,
    run: &DocketJobRunRow,
    task: &DocketTaskInput,
    parent_task_id: Option<Uuid>,
) -> Result<DocketTaskRow, DenError> {
    let row = sqlx::query_as::<_, DocketTaskRow>(
        r"
        INSERT INTO bear_tasks (
            bear_id, job_id, parent_task_id, sibling_order, kind, scope, title, body,
            completion_criteria, difficulty, effort_hint, assigned_to_role, created_by_role, created_by_user_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb, $10, $11, $12, $13, $14)
        RETURNING id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                  kind, scope, title, body, completion_criteria, difficulty, effort_hint, assigned_to_role,
                  created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
                  created_at, updated_at
        ",
    )
    .bind(create.bear_id)
    .bind(job_id)
    .bind(parent_task_id)
    .bind(task.sibling_order)
    .bind(task.kind.as_str())
    .bind(task.scope.as_str())
    .bind(task.title.trim())
    .bind(task.body.trim())
    .bind(serde_json::to_value(normalize_completion_criteria(&task.completion_criteria))?)
    .bind(task.difficulty.map(|difficulty| difficulty.as_str()))
    .bind(task.effort_hint.map(|effort| effort.as_str()))
    .bind(task.assigned_to_role.map(|role| role.as_str()))
    .bind(create.created_by_role.trim())
    .bind(create.created_by_user_id)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        r"
        INSERT INTO bear_task_run_state (run_id, task_id, status)
        VALUES ($1, $2, 'pending')
        ",
    )
    .bind(run.id)
    .bind(row.id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        r"
        INSERT INTO bear_task_events (task_id, run_id, event_type, by_role, by_user_id, payload)
        VALUES ($1, $2, 'created', $3, $4, $5::jsonb)
        ",
    )
    .bind(row.id)
    .bind(run.id)
    .bind(create.created_by_role.trim())
    .bind(create.created_by_user_id)
    .bind(json!({
        "job_id": row.job_id,
        "definition": docket_task_definition_payload(&row),
    }))
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        r"
        INSERT INTO bear_job_events (job_id, run_id, event_type, task_id, by_role, by_user_id, payload)
        VALUES ($1, $2, 'task_added', $3, $4, $5, $6::jsonb)
        ",
    )
    .bind(job_id)
    .bind(run.id)
    .bind(row.id)
    .bind(create.created_by_role.trim())
    .bind(create.created_by_user_id)
    .bind(json!({
        "definition": docket_task_definition_payload(&row),
    }))
    .execute(&mut **tx)
    .await?;

    Ok(row)
}

pub(super) async fn create_task(
    pool: &PgPool,
    create: DocketTaskCreate,
) -> Result<DocketTaskRow, DenError> {
    validate_docket_task_create(&create)?;
    let mut tx = pool.begin().await?;
    let row = insert_task(&mut tx, &create).await?;
    if let Some(run_id) = create.created_in_run_id {
        sqlx::query(
            r"
            INSERT INTO bear_task_run_state (run_id, task_id, status)
            VALUES ($1, $2, 'pending')
            ON CONFLICT (run_id, task_id) DO NOTHING
            ",
        )
        .bind(run_id)
        .bind(row.id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(row)
}

async fn insert_task(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    create: &DocketTaskCreate,
) -> Result<DocketTaskRow, DenError> {
    sqlx::query_as::<_, DocketTaskRow>(
        r"
        INSERT INTO bear_tasks (
            bear_id, job_id, session_anchor_id, parent_task_id, sibling_order, kind, scope,
            title, body, completion_criteria, difficulty, effort_hint, assigned_to_role, created_by_role,
            created_by_user_id, created_by_agent_id, created_in_run_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::jsonb, $11, $12, $13, $14, $15, $16, $17)
        RETURNING id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                  kind, scope, title, body, completion_criteria, difficulty, effort_hint, assigned_to_role,
                  created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
                  created_at, updated_at
        ",
    )
    .bind(create.bear_id)
    .bind(create.job_id)
    .bind(create.session_anchor_id)
    .bind(create.parent_task_id)
    .bind(create.sibling_order)
    .bind(create.kind.as_str())
    .bind(create.scope.as_str())
    .bind(create.title.trim())
    .bind(create.body.trim())
    .bind(serde_json::to_value(normalize_completion_criteria(&create.completion_criteria))?)
    .bind(create.difficulty.map(|difficulty| difficulty.as_str()))
    .bind(create.effort_hint.map(|effort| effort.as_str()))
    .bind(create.assigned_to_role.map(|role| role.as_str()))
    .bind(create.created_by_role.trim())
    .bind(create.created_by_user_id)
    .bind(create.created_by_agent_id.as_deref())
    .bind(create.created_in_run_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

pub(super) async fn list_jobs(
    pool: &PgPool,
    bear_id: Uuid,
    filter: DocketJobListFilter,
) -> Result<Vec<DocketJobRow>, DenError> {
    let limit = if filter.limit <= 0 {
        50
    } else {
        filter.limit.min(200)
    };
    let rows = sqlx::query_as::<_, DocketJobRow>(
        r"
        SELECT id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref, work_surface_id,
               commit_policy, work_branch, status, visibility, current_run_id, created_at, updated_at
        FROM bear_jobs
        WHERE bear_id = $1
        ORDER BY updated_at DESC
        LIMIT $2
        ",
    )
    .bind(bear_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter(|job| filter.include_cancelled || job.status != "cancelled")
        .filter(|job| {
            filter
                .statuses
                .as_ref()
                .map(|statuses| statuses.iter().any(|status| job.status == status.as_str()))
                .unwrap_or(true)
        })
        .collect())
}

pub(super) async fn get_job(
    pool: &PgPool,
    bear_id: Uuid,
    job_id: Uuid,
) -> Result<Option<DocketJobProjection>, DenError> {
    let Some(job) = sqlx::query_as::<_, DocketJobRow>(
        r"
        SELECT id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref, work_surface_id,
               commit_policy, work_branch, status, visibility, current_run_id, created_at, updated_at
        FROM bear_jobs
        WHERE bear_id = $1 AND id = $2
        ",
    )
    .bind(bear_id)
    .bind(job_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let current_run = if let Some(run_id) = job.current_run_id {
        sqlx::query_as::<_, DocketJobRunRow>(
            r"
            SELECT id, job_id, trigger, schedule_ref, state, started_at, finished_at,
                   outcome, created_at, updated_at
            FROM bear_job_runs
            WHERE job_id = $1 AND id = $2
            ",
        )
        .bind(job.id)
        .bind(run_id)
        .fetch_optional(pool)
        .await?
    } else {
        None
    };

    let criteria = sqlx::query_as::<_, DocketJobCriterionRow>(
        r"
        SELECT id, job_id, kind, description, spec, sibling_order, created_at, updated_at
        FROM bear_job_criteria
        WHERE job_id = $1
        ORDER BY sibling_order, created_at
        ",
    )
    .bind(job.id)
    .fetch_all(pool)
    .await?;

    let tasks = sqlx::query_as::<_, DocketTaskRow>(
        r"
        SELECT id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
               kind, scope, title, body, completion_criteria, difficulty, effort_hint, assigned_to_role,
               created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
               created_at, updated_at
        FROM bear_tasks
        WHERE bear_id = $1 AND job_id = $2
        ORDER BY COALESCE(parent_task_id, '00000000-0000-0000-0000-000000000000'::uuid), sibling_order, created_at
        ",
    )
    .bind(bear_id)
    .bind(job.id)
    .fetch_all(pool)
    .await?;

    let (criteria_states, task_states) = if let Some(run) = current_run.as_ref() {
        (
            list_criterion_states(pool, run.id).await?,
            list_task_run_states(pool, run.id).await?,
        )
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(Some(DocketJobProjection {
        job,
        current_run,
        criteria,
        criteria_states,
        tasks,
        task_states,
    }))
}

pub(super) async fn update_job(
    pool: &PgPool,
    update: DocketJobUpdate,
) -> Result<DocketJobProjection, DenError> {
    if update
        .goal
        .as_deref()
        .map(str::trim)
        .is_some_and(str::is_empty)
    {
        return Err(DenError::ValidationError(
            "Docket job goal must not be empty".to_string(),
        ));
    }
    let mut tx = pool.begin().await?;
    let Some(current) = sqlx::query_as::<_, DocketJobRow>(
        r"
        SELECT id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref, work_surface_id,
               commit_policy, work_branch, status, visibility, current_run_id, created_at, updated_at
        FROM bear_jobs
        WHERE bear_id = $1 AND id = $2
        ",
    )
    .bind(update.bear_id)
    .bind(update.job_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Err(DenError::NotFound(format!(
            "Docket job not found: {}",
            update.job_id
        )));
    };
    let job = sqlx::query_as::<_, DocketJobRow>(
        r"
        UPDATE bear_jobs
        SET goal = $3,
            work_surface_ref = $4,
            work_surface_id = $5,
            commit_policy = $6,
            status = $7,
            visibility = $8,
            updated_at = NOW()
        WHERE bear_id = $1 AND id = $2
        RETURNING id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref, work_surface_id,
                  commit_policy, work_branch, status, visibility, current_run_id, created_at, updated_at
        ",
    )
    .bind(update.bear_id)
    .bind(update.job_id)
    .bind(
        update
            .goal
            .as_deref()
            .map(str::trim)
            .unwrap_or(&current.goal),
    )
    .bind(
        update
            .work_surface_ref
            .clone()
            .unwrap_or_else(|| current.work_surface_ref.clone()),
    )
    .bind(
        update
            .work_surface_id
            .unwrap_or(current.work_surface_id),
    )
    .bind(
        update
            .commit_policy
            .map(|policy| policy.map(|policy| policy.as_str().to_string()))
            .unwrap_or_else(|| current.commit_policy.clone()),
    )
    .bind(
        update
            .status
            .map(|status| status.as_str())
            .unwrap_or(&current.status),
    )
    .bind(
        update
            .visibility
            .map(|visibility| visibility.as_str())
            .unwrap_or(&current.visibility),
    )
    .fetch_one(&mut *tx)
    .await?;
    let run_id = job.current_run_id;
    sqlx::query(
        r"
        INSERT INTO bear_job_events (job_id, run_id, event_type, by_role, by_agent_id, by_user_id, payload)
        VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)
        ",
    )
    .bind(job.id)
    .bind(run_id)
    .bind(job_event_type_for_status(update.status))
    .bind(update.actor_role.as_str())
    .bind(update.actor_agent_id.as_deref())
    .bind(update.actor_user_id)
    .bind(json!({
        "status": job.status,
        "goal": job.goal,
        "visibility": job.visibility,
    }))
    .execute(&mut *tx)
    .await?;
    update_run_for_job_status(&mut tx, run_id, update.status).await?;
    tx.commit().await?;
    get_job(pool, update.bear_id, update.job_id)
        .await?
        .ok_or_else(|| DenError::NotFound(format!("Docket job not found: {}", update.job_id)))
}

fn job_event_type_for_status(status: Option<DocketJobStatus>) -> &'static str {
    match status {
        Some(DocketJobStatus::Blocked) => "job_blocked",
        Some(DocketJobStatus::Completed) => "job_completed",
        Some(DocketJobStatus::Cancelled) => "job_cancelled",
        _ => "note_added",
    }
}

async fn update_run_for_job_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: Option<Uuid>,
    status: Option<DocketJobStatus>,
) -> Result<(), DenError> {
    let Some(run_id) = run_id else {
        return Ok(());
    };
    let Some(status) = status else {
        return Ok(());
    };
    let (state, finished) = match status {
        DocketJobStatus::Running => (Some("running"), false),
        DocketJobStatus::Blocked => (Some("paused"), false),
        DocketJobStatus::Completed => (Some("completed"), true),
        DocketJobStatus::Cancelled => (Some("cancelled"), true),
        _ => (None, false),
    };
    if let Some(state) = state {
        sqlx::query(
            r"
            UPDATE bear_job_runs
            SET state = $2,
                started_at = CASE WHEN $2 = 'running' THEN COALESCE(started_at, NOW()) ELSE started_at END,
                finished_at = CASE WHEN $3 THEN COALESCE(finished_at, NOW()) ELSE finished_at END,
                updated_at = NOW()
            WHERE id = $1
            ",
        )
        .bind(run_id)
        .bind(state)
        .bind(finished)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub(super) async fn evaluate_criterion(
    pool: &PgPool,
    update: DocketCriterionStateUpdate,
) -> Result<DocketJobProjection, DenError> {
    let mut tx = pool.begin().await?;
    let exists = sqlx::query_as::<_, (Uuid,)>(
        r"
        SELECT c.id
        FROM bear_job_criteria c
        JOIN bear_jobs j ON j.id = c.job_id
        WHERE j.bear_id = $1 AND c.job_id = $2 AND c.id = $3
        ",
    )
    .bind(update.bear_id)
    .bind(update.job_id)
    .bind(update.criterion_id)
    .fetch_optional(&mut *tx)
    .await?;
    if exists.is_none() {
        return Err(DenError::NotFound(format!(
            "Docket criterion not found: {}",
            update.criterion_id
        )));
    }
    sqlx::query(
        r"
        INSERT INTO bear_job_criteria_state (run_id, criterion_id, status, evaluated_at, evidence, updated_at)
        VALUES ($1, $2, $3, NOW(), $4::jsonb, NOW())
        ON CONFLICT (run_id, criterion_id) DO UPDATE
        SET status = EXCLUDED.status,
            evaluated_at = NOW(),
            evidence = EXCLUDED.evidence,
            updated_at = NOW()
        ",
    )
    .bind(update.run_id)
    .bind(update.criterion_id)
    .bind(update.status.as_str())
    .bind(update.evidence.as_ref())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r"
        INSERT INTO bear_job_events (job_id, run_id, event_type, by_role, by_agent_id, by_user_id, payload)
        VALUES ($1, $2, 'criterion_evaluated', $3, $4, $5, $6::jsonb)
        ",
    )
    .bind(update.job_id)
    .bind(update.run_id)
    .bind(update.actor_role.as_str())
    .bind(update.actor_agent_id.as_deref())
    .bind(update.actor_user_id)
    .bind(json!({
        "criterion_id": update.criterion_id,
        "status": update.status.as_str(),
    }))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_job(pool, update.bear_id, update.job_id)
        .await?
        .ok_or_else(|| DenError::NotFound(format!("Docket job not found: {}", update.job_id)))
}

pub(super) async fn get_active_execution_session(
    pool: &PgPool,
    bear_id: Uuid,
    owner_profile: BearProfile,
    lookup: DocketExecutionLookup,
) -> Result<Option<DocketExecutionSessionRow>, DenError> {
    let row = if let Some(source_conversation_id) = lookup.source_conversation_id {
        sqlx::query_as::<_, DocketExecutionSessionRow>(SELECT_EXECUTION_BY_CONVERSATION)
            .bind(bear_id)
            .bind(owner_profile.as_str())
            .bind(source_conversation_id)
            .fetch_optional(pool)
            .await?
    } else if let Some(session_id) = lookup.session_id {
        sqlx::query_as::<_, DocketExecutionSessionRow>(SELECT_EXECUTION_BY_SESSION)
            .bind(bear_id)
            .bind(owner_profile.as_str())
            .bind(session_id)
            .fetch_optional(pool)
            .await?
    } else if let Some(source_client_session_id) = lookup.source_client_session_id {
        sqlx::query_as::<_, DocketExecutionSessionRow>(SELECT_EXECUTION_BY_ACP_SESSION)
            .bind(bear_id)
            .bind(owner_profile.as_str())
            .bind(source_client_session_id)
            .fetch_optional(pool)
            .await?
    } else {
        None
    };
    Ok(row)
}

const SELECT_EXECUTION_BY_ACP_SESSION: &str = r"
    SELECT id, bear_id, owner_profile, session_id, source_conversation_id, source_client_session_id,
           job_id, run_id, task_id, state, created_at, updated_at
    FROM docket_execution_sessions
    WHERE bear_id = $1 AND owner_profile = $2 AND source_client_session_id = $3
      AND state IN ('active', 'blocked', 'completing', 'paused')
    ORDER BY updated_at DESC
    LIMIT 1
";

const SELECT_EXECUTION_BY_SESSION: &str = r"
    SELECT id, bear_id, owner_profile, session_id, source_conversation_id, source_client_session_id,
           job_id, run_id, task_id, state, created_at, updated_at
    FROM docket_execution_sessions
    WHERE bear_id = $1 AND owner_profile = $2 AND session_id = $3
      AND state IN ('active', 'blocked', 'completing', 'paused')
    ORDER BY updated_at DESC
    LIMIT 1
";

const SELECT_EXECUTION_BY_CONVERSATION: &str = r"
    SELECT id, bear_id, owner_profile, session_id, source_conversation_id, source_client_session_id,
           job_id, run_id, task_id, state, created_at, updated_at
    FROM docket_execution_sessions
    WHERE bear_id = $1 AND owner_profile = $2 AND source_conversation_id = $3
      AND state IN ('active', 'blocked', 'completing', 'paused')
    ORDER BY updated_at DESC
    LIMIT 1
";

pub(super) async fn upsert_execution_session(
    pool: &PgPool,
    upsert: DocketExecutionSessionUpsert,
) -> Result<DocketExecutionSessionRow, DenError> {
    if upsert.session_id.trim().is_empty() {
        return Err(DenError::ValidationError(
            "Docket execution session_id must not be empty".to_string(),
        ));
    }
    sqlx::query_as::<_, DocketExecutionSessionRow>(
        r"
        INSERT INTO docket_execution_sessions (
            bear_id, owner_profile, session_id, source_conversation_id, source_client_session_id,
            job_id, run_id, task_id, state
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (bear_id, owner_profile, session_id)
            WHERE state IN ('active', 'blocked', 'completing', 'paused')
        DO UPDATE SET
            source_conversation_id = EXCLUDED.source_conversation_id,
            source_client_session_id = EXCLUDED.source_client_session_id,
            job_id = EXCLUDED.job_id,
            run_id = EXCLUDED.run_id,
            task_id = EXCLUDED.task_id,
            state = EXCLUDED.state,
            updated_at = NOW()
        RETURNING id, bear_id, owner_profile, session_id, source_conversation_id, source_client_session_id,
                  job_id, run_id, task_id, state, created_at, updated_at
        ",
    )
    .bind(upsert.bear_id)
    .bind(upsert.owner_profile.as_str())
    .bind(upsert.session_id)
    .bind(upsert.source_conversation_id)
    .bind(upsert.source_client_session_id)
    .bind(upsert.job_id)
    .bind(upsert.run_id)
    .bind(upsert.task_id)
    .bind(upsert.state)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

enum ExecutionSessionRef<'a> {
    Explicit(&'a str),
    AcpClientSession(&'a str),
    Conversation(&'a str),
}

impl ExecutionSessionRef<'_> {
    fn into_session_id(self) -> String {
        match self {
            Self::Explicit(value) => value.to_string(),
            Self::AcpClientSession(value) => format!("acp:{value}"),
            Self::Conversation(value) => format!("conversation:{value}"),
        }
    }
}

fn non_empty_ref(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn execution_session_ref(request: &DocketJobExecuteRequest) -> Option<ExecutionSessionRef<'_>> {
    non_empty_ref(&request.source_conversation_id)
        .map(ExecutionSessionRef::Conversation)
        .or_else(|| non_empty_ref(&request.session_id).map(ExecutionSessionRef::Explicit))
        .or_else(|| {
            non_empty_ref(&request.source_client_session_id)
                .map(ExecutionSessionRef::AcpClientSession)
        })
}

fn execution_session_id(request: &DocketJobExecuteRequest) -> Option<String> {
    execution_session_ref(request).map(ExecutionSessionRef::into_session_id)
}

fn execution_session_state_is_active_like(state: &str) -> bool {
    matches!(state, "active" | "blocked" | "completing" | "paused")
}

async fn retire_active_execution_session(
    pool: &PgPool,
    request: &DocketJobExecuteRequest,
    session_id: &str,
    run_id: Uuid,
    task_id: Option<Uuid>,
    state: &str,
) -> Result<bool, DenError> {
    // ponytail: one execution session can have at most one active-like row today via the partial
    // unique index; update all matching rows anyway so future repair/backfill duplicates clear too.
    let result = sqlx::query(
        r"
        UPDATE docket_execution_sessions
        SET source_conversation_id = $4,
            source_client_session_id = $5,
            job_id = $6,
            run_id = $7,
            task_id = $8,
            state = $9,
            updated_at = NOW()
        WHERE bear_id = $1
          AND owner_profile = $2
          AND session_id = $3
          AND state IN ('active', 'blocked', 'completing', 'paused')
        ",
    )
    .bind(request.bear_id)
    .bind(request.actor_role.as_str())
    .bind(session_id)
    .bind(request.source_conversation_id.as_ref())
    .bind(request.source_client_session_id.as_ref())
    .bind(request.job_id)
    .bind(run_id)
    .bind(task_id)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

async fn record_execution_session(
    pool: &PgPool,
    request: &DocketJobExecuteRequest,
    run_id: Uuid,
    task_id: Option<Uuid>,
    state: &str,
) -> Result<(), DenError> {
    let Some(session_id) = execution_session_id(request) else {
        return Ok(());
    };
    let retired_active = if execution_session_state_is_active_like(state) {
        false
    } else {
        retire_active_execution_session(pool, request, &session_id, run_id, task_id, state).await?
    };
    if !retired_active {
        upsert_execution_session(
            pool,
            DocketExecutionSessionUpsert {
                bear_id: request.bear_id,
                owner_profile: request.actor_role,
                session_id: session_id.clone(),
                source_conversation_id: request.source_conversation_id.clone(),
                source_client_session_id: request.source_client_session_id.clone(),
                job_id: request.job_id,
                run_id,
                task_id,
                state: state.to_string(),
            },
        )
        .await?;
    }
    sqlx::query(
        r"
        INSERT INTO bear_job_events (job_id, run_id, event_type, task_id, by_role, by_agent_id, by_user_id, payload)
        VALUES ($1, $2, 'focus_selected', $3, $4, $5, $6, $7::jsonb)
        ",
    )
    .bind(request.job_id)
    .bind(run_id)
    .bind(task_id)
    .bind(request.actor_role.as_str())
    .bind(request.actor_agent_id.as_deref())
    .bind(request.actor_user_id)
    .bind(json!({
        "session_id": session_id,
        "source_conversation_id": request.source_conversation_id,
        "source_client_session_id": request.source_client_session_id,
        "state": state,
    }))
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn execute_job(
    pool: &PgPool,
    request: DocketJobExecuteRequest,
) -> Result<DocketJobExecuteOutcome, DenError> {
    let Some(projection) = get_job(pool, request.bear_id, request.job_id).await? else {
        return Err(DenError::NotFound(format!(
            "Docket job not found: {}",
            request.job_id
        )));
    };
    let Some(run) = projection.current_run.as_ref() else {
        return Err(DenError::ValidationError(
            "Docket job has no current run".to_string(),
        ));
    };

    if let Some(active) = projection
        .task_states
        .iter()
        .find(|state| state.status == "in_progress")
    {
        mark_job_running(pool, &request, run.id).await?;
        record_execution_session(pool, &request, run.id, Some(active.task_id), "active").await?;
        let job = get_job(pool, request.bear_id, request.job_id)
            .await?
            .ok_or_else(|| {
                DenError::NotFound(format!("Docket job not found: {}", request.job_id))
            })?;
        return Ok(DocketJobExecuteOutcome {
            job,
            selected_task_id: Some(active.task_id),
            completed: false,
            blocked: false,
            message: "Job already has an in-progress task.".to_string(),
        });
    }

    if projection
        .task_states
        .iter()
        .any(|state| state.status == "blocked")
    {
        record_execution_session(pool, &request, run.id, None, "blocked").await?;
        let job = update_job(
            pool,
            DocketJobUpdate {
                bear_id: request.bear_id,
                job_id: request.job_id,
                actor_role: request.actor_role,
                actor_user_id: request.actor_user_id,
                actor_agent_id: request.actor_agent_id,
                goal: None,
                work_surface_ref: None,
                work_surface_id: None,
                commit_policy: None,
                status: Some(DocketJobStatus::Blocked),
                visibility: None,
            },
        )
        .await?;
        return Ok(DocketJobExecuteOutcome {
            job,
            selected_task_id: None,
            completed: false,
            blocked: true,
            message: "A task is blocked; job marked blocked.".to_string(),
        });
    }

    let state_by_task = projection
        .task_states
        .iter()
        .map(|state| (state.task_id, state.status.as_str()))
        .collect::<HashMap<_, _>>();
    if let Some(next) = projection.tasks.iter().find(|task| {
        !matches!(
            state_by_task.get(&task.id).copied().unwrap_or("pending"),
            "done" | "cancelled" | "blocked"
        )
    }) {
        mark_job_running(pool, &request, run.id).await?;
        record_execution_session(pool, &request, run.id, Some(next.id), "active").await?;
        update_task(
            pool,
            DocketTaskUpdate {
                bear_id: request.bear_id,
                task_id: next.id,
                actor_role: request.actor_role,
                actor_user_id: request.actor_user_id,
                actor_agent_id: request.actor_agent_id.clone(),
                definition: DocketTaskDefinitionPatch::default(),
                run_state: Some(super::model::DocketTaskRunStateUpdate {
                    run_id: run.id,
                    status: super::model::DocketTaskStatus::InProgress,
                    result_refs: None,
                    result_summary: None,
                }),
            },
        )
        .await?;
        let job = get_job(pool, request.bear_id, request.job_id)
            .await?
            .ok_or_else(|| {
                DenError::NotFound(format!("Docket job not found: {}", request.job_id))
            })?;
        return Ok(DocketJobExecuteOutcome {
            job,
            selected_task_id: Some(next.id),
            completed: false,
            blocked: false,
            message: "Selected next pending task for pair execution.".to_string(),
        });
    }

    let criteria_complete = projection.criteria.is_empty()
        || projection.criteria.iter().all(|criterion| {
            projection
                .criteria_states
                .iter()
                .find(|state| state.criterion_id == criterion.id)
                .map(|state| matches!(state.status.as_str(), "met" | "waived"))
                .unwrap_or(false)
        });
    if criteria_complete {
        record_execution_session(pool, &request, run.id, None, "completed").await?;
        let job = update_job(
            pool,
            DocketJobUpdate {
                bear_id: request.bear_id,
                job_id: request.job_id,
                actor_role: request.actor_role,
                actor_user_id: request.actor_user_id,
                actor_agent_id: request.actor_agent_id,
                goal: None,
                work_surface_ref: None,
                work_surface_id: None,
                commit_policy: None,
                status: Some(DocketJobStatus::Completed),
                visibility: None,
            },
        )
        .await?;
        Ok(DocketJobExecuteOutcome {
            job,
            selected_task_id: None,
            completed: true,
            blocked: false,
            message: "All tasks and criteria are complete; job completed.".to_string(),
        })
    } else {
        record_execution_session(pool, &request, run.id, None, "blocked").await?;
        let job = update_job(
            pool,
            DocketJobUpdate {
                bear_id: request.bear_id,
                job_id: request.job_id,
                actor_role: request.actor_role,
                actor_user_id: request.actor_user_id,
                actor_agent_id: request.actor_agent_id,
                goal: None,
                work_surface_ref: None,
                work_surface_id: None,
                commit_policy: None,
                status: Some(DocketJobStatus::Blocked),
                visibility: None,
            },
        )
        .await?;
        Ok(DocketJobExecuteOutcome {
            job,
            selected_task_id: None,
            completed: false,
            blocked: true,
            message: "All tasks are terminal, but acceptance criteria are not met.".to_string(),
        })
    }
}

async fn mark_job_running(
    pool: &PgPool,
    request: &DocketJobExecuteRequest,
    run_id: Uuid,
) -> Result<(), DenError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r"
        UPDATE bear_jobs
        SET status = 'running', updated_at = NOW()
        WHERE bear_id = $1 AND id = $2 AND status <> 'running'
        ",
    )
    .bind(request.bear_id)
    .bind(request.job_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r"
        UPDATE bear_job_runs
        SET state = 'running', started_at = COALESCE(started_at, NOW()), updated_at = NOW()
        WHERE id = $1
        ",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r"
        INSERT INTO bear_job_events (job_id, run_id, event_type, by_role, by_agent_id, by_user_id, payload)
        VALUES ($1, $2, 'run_started', $3, $4, $5, $6::jsonb)
        ",
    )
    .bind(request.job_id)
    .bind(run_id)
    .bind(request.actor_role.as_str())
    .bind(request.actor_agent_id.as_deref())
    .bind(request.actor_user_id)
    .bind(json!({"status": "running"}))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn list_criterion_states(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<DocketCriterionStateRow>, DenError> {
    sqlx::query_as::<_, DocketCriterionStateRow>(
        r"
        SELECT run_id, criterion_id, status, evaluated_at, evidence, updated_at
        FROM bear_job_criteria_state
        WHERE run_id = $1
        ORDER BY updated_at DESC
        ",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn list_task_run_states(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<DocketTaskRunStateRow>, DenError> {
    sqlx::query_as::<_, DocketTaskRunStateRow>(
        r"
        SELECT run_id, task_id, status, result_refs, result_summary, started_at, finished_at, updated_at
        FROM bear_task_run_state
        WHERE run_id = $1
        ORDER BY updated_at DESC
        ",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub(super) async fn list_tasks(
    pool: &PgPool,
    bear_id: Uuid,
    filter: DocketTaskListFilter,
) -> Result<Vec<DocketTaskProjection>, DenError> {
    let limit = if filter.limit <= 0 {
        100
    } else {
        filter.limit.min(500)
    };
    let tasks = if filter.include_descendants {
        list_tasks_with_descendants(pool, bear_id, &filter, limit).await?
    } else {
        sqlx::query_as::<_, DocketTaskRow>(
            r"
            SELECT id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                   kind, scope, title, body, completion_criteria, difficulty, effort_hint, assigned_to_role,
                   created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
                   created_at, updated_at
            FROM bear_tasks
            WHERE bear_id = $1
              AND ($2::uuid IS NULL OR job_id = $2)
              AND ($3::uuid IS NULL OR session_anchor_id = $3)
              AND (
                    ($4::uuid IS NULL AND parent_task_id IS NULL)
                 OR ($4::uuid IS NOT NULL AND parent_task_id = $4)
              )
            ORDER BY sibling_order, created_at
            LIMIT $5
            ",
        )
        .bind(bear_id)
        .bind(filter.job_id)
        .bind(filter.session_anchor_id)
        .bind(filter.parent_task_id)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    let states = current_run_states_for_tasks(pool, filter.job_id, &tasks).await?;
    Ok(tasks
        .into_iter()
        .map(|task| DocketTaskProjection {
            run_state: states.get(&task.id).cloned(),
            task,
        })
        .collect())
}

async fn list_tasks_with_descendants(
    pool: &PgPool,
    bear_id: Uuid,
    filter: &DocketTaskListFilter,
    limit: i64,
) -> Result<Vec<DocketTaskRow>, DenError> {
    sqlx::query_as::<_, DocketTaskRow>(
        r"
        WITH RECURSIVE task_tree AS (
            SELECT id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                   kind, scope, title, body, completion_criteria, difficulty, effort_hint, assigned_to_role,
                   created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
                   created_at, updated_at
            FROM bear_tasks
            WHERE bear_id = $1
              AND ($2::uuid IS NULL OR job_id = $2)
              AND ($3::uuid IS NULL OR session_anchor_id = $3)
              AND (
                    ($4::uuid IS NULL AND parent_task_id IS NULL)
                 OR ($4::uuid IS NOT NULL AND parent_task_id = $4)
              )
            UNION ALL
            SELECT child.id, child.bear_id, child.job_id, child.session_anchor_id,
                   child.parent_task_id, child.sibling_order, child.kind, child.scope,
                   child.title, child.body, child.completion_criteria, child.difficulty, child.effort_hint,
                   child.assigned_to_role, child.created_by_role, child.created_by_user_id,
                   child.created_by_agent_id, child.created_in_run_id, child.created_at,
                   child.updated_at
            FROM bear_tasks child
            JOIN task_tree parent ON child.parent_task_id = parent.id
        )
        SELECT id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
               kind, scope, title, body, completion_criteria, difficulty, effort_hint, assigned_to_role,
               created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
               created_at, updated_at
        FROM task_tree
        ORDER BY COALESCE(parent_task_id, '00000000-0000-0000-0000-000000000000'::uuid), sibling_order, created_at
        LIMIT $5
        ",
    )
    .bind(bear_id)
    .bind(filter.job_id)
    .bind(filter.session_anchor_id)
    .bind(filter.parent_task_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn current_run_states_for_tasks(
    pool: &PgPool,
    job_id: Option<Uuid>,
    tasks: &[DocketTaskRow],
) -> Result<HashMap<Uuid, DocketTaskRunStateRow>, DenError> {
    if tasks.is_empty() {
        return Ok(HashMap::new());
    }

    if let Some(job_id) = job_id.or_else(|| tasks.iter().find_map(|task| task.job_id)) {
        let run_id = sqlx::query_as::<_, (Option<Uuid>,)>(
            r"SELECT current_run_id FROM bear_jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_optional(pool)
        .await?
        .and_then(|row| row.0);
        if let Some(run_id) = run_id {
            return Ok(list_task_run_states(pool, run_id)
                .await?
                .into_iter()
                .map(|state| (state.task_id, state))
                .collect());
        }
    }

    // ponytail: session-anchored tasks do not have a job current_run_id to join
    // through. Use the latest recorded state per task; if session tasks ever
    // support multiple simultaneously visible runs, thread the desired run id
    // through DocketTaskListFilter instead.
    let task_ids: Vec<Uuid> = tasks.iter().map(|task| task.id).collect();
    sqlx::query_as::<_, DocketTaskRunStateRow>(
        r"
        SELECT DISTINCT ON (task_id)
               run_id, task_id, status, result_refs, result_summary, started_at, finished_at, updated_at
        FROM bear_task_run_state
        WHERE task_id = ANY($1)
        ORDER BY task_id, updated_at DESC
        ",
    )
    .bind(&task_ids)
    .fetch_all(pool)
    .await
    .map(|states| {
        states
            .into_iter()
            .map(|state| (state.task_id, state))
            .collect()
    })
    .map_err(Into::into)
}

pub(super) async fn update_task(
    pool: &PgPool,
    update: DocketTaskUpdate,
) -> Result<DocketTaskProjection, DenError> {
    validate_docket_task_patch(&update.definition)?;
    validate_docket_task_run_state_update(update.run_state.as_ref())?;
    let mut tx = pool.begin().await?;
    let current = select_task(&mut tx, update.bear_id, update.task_id).await?;
    let patched = update_task_definition(&mut tx, &current, &update.definition).await?;
    append_task_updated_events(&mut tx, &patched, &update).await?;
    let run_state = if let Some(run_state) = update.run_state.as_ref() {
        Some(upsert_task_run_state(&mut tx, update.task_id, run_state).await?)
    } else {
        None
    };
    tx.commit().await?;
    Ok(DocketTaskProjection {
        task: patched,
        run_state,
    })
}

fn validate_docket_task_run_state_update(
    update: Option<&super::model::DocketTaskRunStateUpdate>,
) -> Result<(), DenError> {
    let Some(update) = update else {
        return Ok(());
    };
    if update.status.as_str() == "done"
        && update
            .result_summary
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
    {
        return Err(DenError::ValidationError(
            "Docket task completion requires non-empty result_summary".to_string(),
        ));
    }
    Ok(())
}

fn validate_docket_task_patch(patch: &DocketTaskDefinitionPatch) -> Result<(), DenError> {
    if let Some(criteria) = patch.completion_criteria.as_ref() {
        super::model::validate_completion_criteria(criteria)?;
    }
    if patch
        .title
        .as_deref()
        .map(str::trim)
        .is_some_and(str::is_empty)
    {
        return Err(DenError::ValidationError(
            "Docket task title must not be empty".to_string(),
        ));
    }
    if patch
        .body
        .as_deref()
        .map(str::trim)
        .is_some_and(str::is_empty)
    {
        return Err(DenError::ValidationError(
            "Docket task body must not be empty".to_string(),
        ));
    }
    Ok(())
}

async fn select_task(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    bear_id: Uuid,
    task_id: Uuid,
) -> Result<DocketTaskRow, DenError> {
    sqlx::query_as::<_, DocketTaskRow>(
        r"
        SELECT id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
               kind, scope, title, body, completion_criteria, difficulty, effort_hint, assigned_to_role,
               created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
               created_at, updated_at
        FROM bear_tasks
        WHERE bear_id = $1 AND id = $2
        ",
    )
    .bind(bear_id)
    .bind(task_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DenError::NotFound(format!("Docket task not found: {task_id}")))
}

async fn update_task_definition(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    current: &DocketTaskRow,
    patch: &DocketTaskDefinitionPatch,
) -> Result<DocketTaskRow, DenError> {
    sqlx::query_as::<_, DocketTaskRow>(
        r"
        UPDATE bear_tasks
        SET title = $3,
            body = $4,
            completion_criteria = $5::jsonb,
            parent_task_id = $6,
            sibling_order = $7,
            kind = $8,
            scope = $9,
            difficulty = $10,
            effort_hint = $11,
            assigned_to_role = $12,
            updated_at = NOW()
        WHERE bear_id = $1 AND id = $2
        RETURNING id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                  kind, scope, title, body, completion_criteria, difficulty, effort_hint, assigned_to_role,
                  created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
                  created_at, updated_at
        ",
    )
    .bind(current.bear_id)
    .bind(current.id)
    .bind(
        patch
            .title
            .as_deref()
            .map(str::trim)
            .unwrap_or(&current.title),
    )
    .bind(
        patch
            .body
            .as_deref()
            .map(str::trim)
            .unwrap_or(&current.body),
    )
    .bind(serde_json::to_value(
        patch
            .completion_criteria
            .as_ref()
            .map(|criteria| normalize_completion_criteria(criteria))
            .unwrap_or_else(|| current.completion_criteria.0.clone()),
    )?)
    .bind(patch.parent_task_id.unwrap_or(current.parent_task_id))
    .bind(patch.sibling_order.unwrap_or(current.sibling_order))
    .bind(
        patch
            .kind
            .map(|kind| kind.as_str())
            .unwrap_or(&current.kind),
    )
    .bind(
        patch
            .scope
            .map(|scope| scope.as_str())
            .unwrap_or(&current.scope),
    )
    .bind(
        patch
            .difficulty
            .map(|value| value.map(|difficulty| difficulty.as_str().to_string()))
            .unwrap_or_else(|| current.difficulty.clone()),
    )
    .bind(
        patch
            .effort_hint
            .map(|value| value.map(|effort| effort.as_str().to_string()))
            .unwrap_or_else(|| current.effort_hint.clone()),
    )
    .bind(
        patch
            .assigned_to_role
            .map(|value| value.map(|role| role.as_str().to_string()))
            .unwrap_or_else(|| current.assigned_to_role.clone()),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

async fn append_task_updated_events(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &DocketTaskRow,
    update: &DocketTaskUpdate,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        INSERT INTO bear_task_events (task_id, run_id, event_type, by_role, by_agent_id, by_user_id, payload)
        VALUES ($1, $2, 'updated', $3, $4, $5, $6::jsonb)
        ",
    )
    .bind(task.id)
    .bind(update.run_state.as_ref().map(|state| state.run_id))
    .bind(update.actor_role.as_str())
    .bind(update.actor_agent_id.as_deref())
    .bind(update.actor_user_id)
    .bind(json!({
        "definition": docket_task_definition_payload(task),
    }))
    .execute(&mut **tx)
    .await?;

    if let Some(job_id) = task.job_id {
        sqlx::query(
            r"
            INSERT INTO bear_job_events (job_id, run_id, event_type, task_id, by_role, by_agent_id, by_user_id, payload)
            VALUES ($1, $2, 'task_updated', $3, $4, $5, $6, $7::jsonb)
            ",
        )
        .bind(job_id)
        .bind(update.run_state.as_ref().map(|state| state.run_id))
        .bind(task.id)
        .bind(update.actor_role.as_str())
        .bind(update.actor_agent_id.as_deref())
        .bind(update.actor_user_id)
        .bind(json!({
            "title": task.title,
            "parent_task_id": task.parent_task_id,
            "scope": task.scope,
        }))
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn upsert_task_run_state(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task_id: Uuid,
    update: &super::model::DocketTaskRunStateUpdate,
) -> Result<DocketTaskRunStateRow, DenError> {
    if update.status.as_str() == "in_progress" {
        sqlx::query(
            r"
            UPDATE bear_task_run_state
            SET status = 'pending', updated_at = NOW()
            WHERE run_id = $1 AND task_id <> $2 AND status = 'in_progress'
            ",
        )
        .bind(update.run_id)
        .bind(task_id)
        .execute(&mut **tx)
        .await?;
    }

    sqlx::query_as::<_, DocketTaskRunStateRow>(
        r"
        INSERT INTO bear_task_run_state (
            run_id, task_id, status, result_refs, result_summary, started_at, finished_at, updated_at
        )
        VALUES (
            $1, $2, $3, $4::jsonb, $5,
            CASE WHEN $3 = 'in_progress' THEN COALESCE(NOW(), NOW()) ELSE NULL END,
            CASE WHEN $3 IN ('done', 'cancelled') THEN NOW() ELSE NULL END,
            NOW()
        )
        ON CONFLICT (run_id, task_id) DO UPDATE
        SET status = EXCLUDED.status,
            result_refs = EXCLUDED.result_refs,
            result_summary = EXCLUDED.result_summary,
            started_at = CASE
                WHEN EXCLUDED.status = 'in_progress' THEN COALESCE(bear_task_run_state.started_at, NOW())
                ELSE bear_task_run_state.started_at
            END,
            finished_at = CASE
                WHEN EXCLUDED.status IN ('done', 'cancelled') THEN COALESCE(bear_task_run_state.finished_at, NOW())
                WHEN EXCLUDED.status IN ('pending', 'in_progress', 'blocked') THEN NULL
                ELSE bear_task_run_state.finished_at
            END,
            updated_at = NOW()
        RETURNING run_id, task_id, status, result_refs, result_summary, started_at, finished_at, updated_at
        ",
    )
    .bind(update.run_id)
    .bind(task_id)
    .bind(update.status.as_str())
    .bind(update.result_refs.as_ref())
    .bind(update.result_summary.as_deref())
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

pub(super) async fn sync_task_list(
    pool: &PgPool,
    request: TaskListSyncRequest,
) -> Result<TaskListSyncOutcome, DenError> {
    let Some(job_id) = task_list_job_id(&request.task_list) else {
        return Ok(TaskListSyncOutcome::review_required(
            request.task_list,
            "Task list is not Docket-backed; request handoff/promotion before syncing.",
        ));
    };
    let Some(job) = get_job(pool, request.task_list.bear_id, job_id).await? else {
        return Ok(TaskListSyncOutcome::conflicts(
            request.task_list,
            vec![format!("Docket job not found: {job_id}")],
            "Task-list sync could not find its Docket job.",
        ));
    };
    let Some(run_id) = job.job.current_run_id else {
        return Ok(TaskListSyncOutcome::conflicts(
            request.task_list,
            vec![format!("Docket job has no current run: {job_id}")],
            "Task-list sync requires a current Docket run for status updates.",
        ));
    };

    let tasks_by_id = job
        .tasks
        .iter()
        .map(|task| (task.id, task))
        .collect::<HashMap<_, _>>();
    let parent_task_id = docket_parent_task_ref(&request.task_list.source_ref);
    let mut conflicts = Vec::new();
    for item in &request.task_list.items {
        if let Some(task_id) = task_ref_uuid(&item.source_ref) {
            let Some(existing) = tasks_by_id.get(&task_id).copied() else {
                conflicts.push(format!(
                    "Docket task not found for item `{}`: {task_id}",
                    item.id
                ));
                continue;
            };
            if existing.updated_at > request.task_list.updated_at
                && (existing.title != item.title
                    || item
                        .summary
                        .as_deref()
                        .is_some_and(|summary| summary != existing.body))
            {
                conflicts.push(format!(
                    "Docket task `{}` changed after checkout; refresh before syncing item `{}`",
                    existing.id, item.id
                ));
            }
            if item.status == TaskListItemStatus::Completed
                && item
                    .summary
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(|summary| summary.is_empty() || summary == existing.body.trim())
            {
                conflicts.push(format!(
                    "Completed Docket-backed item `{}` requires a completion summary/evidence distinct from the task body",
                    item.id
                ));
            }
        }
    }
    if !conflicts.is_empty() {
        return Ok(TaskListSyncOutcome::conflicts(
            request.task_list,
            conflicts,
            "Task-list sync found conflicts; refresh checkout and reconcile before applying.",
        ));
    }

    for item in &request.task_list.items {
        if matches!(
            item.sync_state,
            TaskListSyncState::Conflict | TaskListSyncState::ReviewRequired
        ) {
            continue;
        }
        if let Some(task_id) = task_ref_uuid(&item.source_ref) {
            let existing = tasks_by_id.get(&task_id).copied();
            let body = item.summary.clone();
            let result_summary = match item.status {
                TaskListItemStatus::Completed => item
                    .summary
                    .as_ref()
                    .filter(|summary| {
                        existing
                            .map(|task| task.body.trim() != summary.trim())
                            .unwrap_or(true)
                    })
                    .cloned(),
                TaskListItemStatus::Blocked => item.blocked_reason.clone(),
                _ => None,
            };
            update_task(
                pool,
                DocketTaskUpdate {
                    bear_id: request.task_list.bear_id,
                    task_id,
                    actor_role: request
                        .task_list
                        .owner_profile
                        .parse()
                        .map_err(DenError::Parsing)?,
                    actor_user_id: None,
                    actor_agent_id: None,
                    definition: DocketTaskDefinitionPatch {
                        title: Some(item.title.clone()),
                        body,
                        ..DocketTaskDefinitionPatch::default()
                    },
                    run_state: Some(super::model::DocketTaskRunStateUpdate {
                        run_id,
                        status: docket_task_status_from_task_list_item_status(item.status),
                        result_refs: None,
                        result_summary,
                    }),
                },
            )
            .await?;
        } else if item.source_ref.kind == "local" {
            create_task(
                pool,
                DocketTaskCreate {
                    bear_id: request.task_list.bear_id,
                    job_id: Some(job_id),
                    session_anchor_id: None,
                    parent_task_id,
                    sibling_order: i32::MAX / 2,
                    kind: super::model::DocketTaskKind::Execution,
                    scope: super::model::DocketTaskScope::Template,
                    title: item.title.clone(),
                    body: item.summary.clone().unwrap_or_else(|| item.title.clone()),
                    completion_criteria: vec![item
                        .summary
                        .clone()
                        .unwrap_or_else(|| format!("Complete: {}", item.title))],
                    difficulty: None,
                    effort_hint: None,
                    assigned_to_role: Some(BearProfile::Work),
                    created_by_role: request.task_list.owner_profile.clone(),
                    created_by_user_id: None,
                    created_by_agent_id: None,
                    created_in_run_id: Some(run_id),
                },
            )
            .await?;
        }
    }

    let refreshed = get_job(pool, request.task_list.bear_id, job_id)
        .await?
        .map(|job| task_list_projection_from_docket_job(&job, parent_task_id))
        .unwrap_or(request.task_list);
    Ok(TaskListSyncOutcome::applied(
        refreshed,
        "Task-list changes synced to Docket.",
    ))
}

fn task_list_job_id(task_list: &TaskListProjection) -> Option<Uuid> {
    task_list
        .source_ref
        .docket_job_id
        .as_deref()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .or_else(|| {
            task_list.items.iter().find_map(|item| {
                item.source_ref
                    .docket_job_id
                    .as_deref()
                    .and_then(|raw| Uuid::parse_str(raw).ok())
            })
        })
}

fn task_ref_uuid(source_ref: &TaskListSourceRef) -> Option<Uuid> {
    source_ref
        .docket_task_id
        .as_deref()
        .and_then(|raw| Uuid::parse_str(raw).ok())
}
