//! Docket Postgres access — internal to the module.
//!
//! These functions are the persistence layer behind `DocketService`; callers
//! outside Docket go through the service, never here. Docket job/task data is
//! stored in the ADR-0034 relational tables.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use den_core::{BearProfile, DenError};

use super::model::{
    derived_docket_job_status, docket_job_surface_assignments, docket_parent_task_ref,
    docket_task_status_from_task_list_item_status, normalize_completion_criteria,
    task_list_projection_from_docket_job, validate_docket_job_create, validate_docket_task_create,
    DocketCommitPolicy, DocketCriterionStateRow, DocketCriterionStateUpdate, DocketEntryCreate,
    DocketEntryKind, DocketEntryListFilter, DocketEntryPromotion, DocketEntryRow, DocketEntryScope,
    DocketExecutionLookup, DocketExecutionSessionRow, DocketExecutionSessionUpsert,
    DocketJobCreate, DocketJobCriterionRow, DocketJobExecuteOutcome, DocketJobExecuteRequest,
    DocketJobListFilter, DocketJobProjection, DocketJobRow, DocketJobRunRow, DocketJobStatus,
    DocketJobUpdate, DocketSessionTaskSettlement, DocketTaskCreate, DocketTaskDefinitionPatch,
    DocketTaskInput, DocketTaskListFilter, DocketTaskPlacement, DocketTaskProjection,
    DocketTaskRow, DocketTaskRunStateRow, DocketTaskUpdate, DocketValidationError,
    TaskListItemStatus, TaskListProjection, TaskListSourceRef, TaskListSyncOutcome,
    TaskListSyncRequest, TaskListSyncState,
};

// `work_surface_id` remains a compatibility projection for callers that only
// understand one Git workspace. Assignments are the canonical relationship.
const JOB_COLUMNS: &str = "j.id, j.bear_id, j.created_by_user_id, j.created_by_role, j.goal, \
    (SELECT a.work_surface_id FROM job_work_surface_assignments a \
     JOIN work_surfaces s ON s.id = a.work_surface_id \
     WHERE a.job_id = j.id AND s.kind = 'git_workspace' AND a.mutation_policy <> 'forbidden' \
     ORDER BY a.created_at LIMIT 1) AS work_surface_id, \
    j.commit_policy, j.work_branch, COALESCE(j.lifecycle_intent, 'draft') AS status, \
    j.lifecycle_intent, j.visibility, j.source_conversation_id, j.objective_kind, \
    j.supersedes_job_id, j.current_run_id, j.created_at, j.updated_at";

pub(super) async fn create_job(
    pool: &PgPool,
    create: DocketJobCreate,
) -> Result<DocketJobProjection, DenError> {
    validate_docket_job_create(&create)?;
    let surface_assignments = docket_job_surface_assignments(&create);
    if matches!(
        create.overlap_resolution,
        super::model::DocketJobOverlapResolution::Supersede
    ) && create.supersedes_job_id.is_none()
    {
        return Err(DocketValidationError::SupersedeRequiresPredecessor.into());
    }

    let mut tx = pool.begin().await?;
    let predecessor = sqlx::query_scalar::<_, Uuid>(
        r"
        SELECT j.id
        FROM bear_jobs j
        WHERE j.bear_id = $1
          AND j.lifecycle_intent IS NULL
          AND lower(btrim(j.goal)) = lower(btrim($2))
          AND EXISTS (
              SELECT 1 FROM job_work_surface_assignments a
              WHERE a.job_id = j.id AND a.work_surface_id = $3
          )
          AND ($4::uuid IS NULL OR j.id = $4)
        ORDER BY j.created_at DESC
        LIMIT 1
        FOR UPDATE
        ",
    )
    .bind(create.bear_id)
    .bind(create.goal.trim())
    .bind(create.work_surface_id)
    .bind(create.supersedes_job_id)
    .fetch_optional(&mut *tx)
    .await?;

    match (predecessor, create.overlap_resolution) {
        (Some(job_id), super::model::DocketJobOverlapResolution::Reject) => {
            return Err(DocketValidationError::ActiveJobOverlap { job_id }.into());
        }
        (Some(job_id), super::model::DocketJobOverlapResolution::Supersede)
            if create.supersedes_job_id == Some(job_id) =>
        {
            sqlx::query(
                "UPDATE bear_jobs SET lifecycle_intent = 'cancelled', updated_at = NOW() WHERE id = $1",
            )
            .bind(job_id)
            .execute(&mut *tx)
            .await?;
        }
        (Some(job_id), super::model::DocketJobOverlapResolution::Supersede) => {
            return Err(
                DocketValidationError::SupersedeRequiresMatchingActiveJob { job_id }.into(),
            );
        }
        (None, super::model::DocketJobOverlapResolution::Supersede) => {
            return Err(DocketValidationError::SupersedeRequiresMatchingActiveJob {
                job_id: create.supersedes_job_id.expect("validated above"),
            }
            .into());
        }
        (_, super::model::DocketJobOverlapResolution::Independent) | (None, _) => {}
    }

    let job_id: Uuid = sqlx::query_scalar(
        r"
        INSERT INTO bear_jobs (
            bear_id, created_by_user_id, created_by_role, goal,
            commit_policy, work_branch, lifecycle_intent, visibility, source_conversation_id, objective_kind,
            supersedes_job_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id
        ",
    )
    .bind(create.bear_id)
    .bind(create.created_by_user_id)
    .bind(create.created_by_role.trim())
    .bind(create.goal.trim())
    .bind(DocketCommitPolicy::for_new_job(create.commit_policy).as_str())
    .bind(
        create
            .work_branch
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty()),
    )
    .bind(Option::<&str>::None)
    .bind(create.visibility.as_str())
    .bind(create.source_conversation_id.as_deref())
    .bind(create.objective_kind.as_deref())
    .bind(create.supersedes_job_id)
    .fetch_one(&mut *tx)
    .await?;

    for assignment in &surface_assignments {
        sqlx::query(
            r"
            INSERT INTO job_work_surface_assignments (job_id, work_surface_id, mutation_policy)
            VALUES ($1, $2, $3)
            ",
        )
        .bind(job_id)
        .bind(assignment.work_surface_id)
        .bind(assignment.mutation_policy.as_str())
        .execute(&mut *tx)
        .await?;
    }

    let run = sqlx::query_as::<_, DocketJobRunRow>(
        r"
        INSERT INTO bear_job_runs (job_id, trigger, state)
        VALUES ($1, 'manual', 'dispatched')
        RETURNING id, job_id, trigger, schedule_ref, state, started_at, finished_at,
                  outcome, created_at, updated_at
        ",
    )
    .bind(job_id)
    .fetch_one(&mut *tx)
    .await?;

    let job = sqlx::query_as::<_, DocketJobRow>(&format!(
        r"
        UPDATE bear_jobs j
        SET current_run_id = $2, updated_at = NOW()
        WHERE j.id = $1
        RETURNING {JOB_COLUMNS}
        ",
    ))
    .bind(job_id)
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
    for (index, task) in create.tasks.iter().enumerate() {
        let parent_task_id = resolve_parent_task_id(task, &task_ids_by_client_key)?;
        let sibling_order = task
            .sibling_order
            .unwrap_or_else(|| i32::try_from(index).unwrap_or(i32::MAX));
        let row = insert_task_for_job(
            &mut tx,
            &create,
            job.id,
            &run,
            task,
            parent_task_id,
            sibling_order,
        )
        .await?;
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
        "lifecycle_intent": job.lifecycle_intent,
        "visibility": job.visibility,
    }))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    let task_states = list_task_run_states(pool, run.id).await?;
    let criteria_states = list_criterion_states(pool, run.id).await?;
    let mut projection = DocketJobProjection {
        job,
        current_run: Some(run),
        criteria,
        criteria_states,
        tasks,
        task_states,
        active_task_ids: Vec::new(),
    };
    projection.job.status = derived_docket_job_status(&projection);
    Ok(projection)
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
        "routing_strategy": task.routing_strategy,
        "expected_context_size": task.expected_context_size,
        "result_rollup_policy": task.result_rollup_policy,
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
    sibling_order: i32,
) -> Result<DocketTaskRow, DenError> {
    let row = sqlx::query_as::<_, DocketTaskRow>(
        r"
        INSERT INTO bear_tasks (
            bear_id, job_id, parent_task_id, sibling_order, kind, scope, title, body,
            completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
                  result_rollup_policy, created_by_role, created_by_user_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb, $10, $11, $12, $13, $14, $15, $16)
        RETURNING id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                  kind, scope, title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
                  result_rollup_policy, created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
                  created_at, updated_at
        ",
    )
    .bind(create.bear_id)
    .bind(job_id)
    .bind(parent_task_id)
    .bind(sibling_order)
    .bind(task.kind.as_str())
    .bind(task.scope.as_str())
    .bind(task.title.trim())
    .bind(task.body.trim())
    .bind(serde_json::to_value(normalize_completion_criteria(
        &task.completion_criteria,
    ))?)
    .bind(task.difficulty.map(|difficulty| difficulty.as_str()))
    .bind(task.effort_hint.map(|effort| effort.as_str()))
    .bind(task.routing_strategy.as_str())
    .bind(task.expected_context_size)
    .bind(task.result_rollup_policy.map(|policy| policy.as_str()))
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
    let sibling_order = place_task(&mut tx, &create).await?;
    let mut create = create;
    create.sibling_order = sibling_order;
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

async fn place_task(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    create: &DocketTaskCreate,
) -> Result<i32, DenError> {
    // Serialize placement within a task tree; jobs and session anchors are the
    // stable roots available before a top-level task exists.
    if let Some(job_id) = create.job_id {
        sqlx::query("SELECT id FROM bear_jobs WHERE id = $1 FOR UPDATE")
            .bind(job_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| DenError::ValidationError("Docket job not found".to_string()))?;
    } else if let Some(session_anchor_id) = create.session_anchor_id {
        sqlx::query("SELECT id FROM client_sessions WHERE id = $1 FOR UPDATE")
            .bind(session_anchor_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                DenError::ValidationError("task session anchor not found".to_string())
            })?;
    }

    let placement = create.placement.unwrap_or(DocketTaskPlacement::Last);
    let target_order = match placement {
        DocketTaskPlacement::First => 0,
        DocketTaskPlacement::Last => {
            sqlx::query_scalar::<_, i32>(
                r"
            SELECT COALESCE(MAX(sibling_order), -1) + 1
            FROM bear_tasks
            WHERE bear_id = $1
              AND job_id IS NOT DISTINCT FROM $2
              AND session_anchor_id IS NOT DISTINCT FROM $3
              AND parent_task_id IS NOT DISTINCT FROM $4
            ",
            )
            .bind(create.bear_id)
            .bind(create.job_id)
            .bind(create.session_anchor_id)
            .bind(create.parent_task_id)
            .fetch_one(&mut **tx)
            .await?
        }
        DocketTaskPlacement::Before { task_id } | DocketTaskPlacement::After { task_id } => {
            let anchor: DocketTaskRow = sqlx::query_as(
                r"
                SELECT id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                       kind, scope, title, body, completion_criteria, difficulty, effort_hint,
                       routing_strategy, expected_context_size, result_rollup_policy,
                       created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
                       created_at, updated_at
                FROM bear_tasks
                WHERE id = $1 AND bear_id = $2
                ",
            )
            .bind(task_id)
            .bind(create.bear_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                DenError::ValidationError("placement anchor task not found".to_string())
            })?;
            if anchor.job_id != create.job_id
                || anchor.session_anchor_id != create.session_anchor_id
                || anchor.parent_task_id != create.parent_task_id
            {
                return Err(DenError::ValidationError(
                    "placement anchor must be a sibling in the same task tree".to_string(),
                ));
            }
            anchor.sibling_order + i32::from(matches!(placement, DocketTaskPlacement::After { .. }))
        }
    };

    sqlx::query(
        r"
        UPDATE bear_tasks
        SET sibling_order = sibling_order + 1, updated_at = NOW()
        WHERE bear_id = $1
          AND job_id IS NOT DISTINCT FROM $2
          AND session_anchor_id IS NOT DISTINCT FROM $3
          AND parent_task_id IS NOT DISTINCT FROM $4
          AND sibling_order >= $5
        ",
    )
    .bind(create.bear_id)
    .bind(create.job_id)
    .bind(create.session_anchor_id)
    .bind(create.parent_task_id)
    .bind(target_order)
    .execute(&mut **tx)
    .await?;

    Ok(target_order)
}

async fn insert_task(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    create: &DocketTaskCreate,
) -> Result<DocketTaskRow, DenError> {
    sqlx::query_as::<_, DocketTaskRow>(
        r"
        INSERT INTO bear_tasks (
            bear_id, job_id, session_anchor_id, parent_task_id, sibling_order, kind, scope,
            title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
                  result_rollup_policy, created_by_role,
            created_by_user_id, created_by_agent_id, created_in_run_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::jsonb, $11, $12, $13, $14, $15, $16, $17, $18, $19)
        RETURNING id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                  kind, scope, title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
                  result_rollup_policy, created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
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
    .bind(serde_json::to_value(normalize_completion_criteria(
        &create.completion_criteria,
    ))?)
    .bind(create.difficulty.map(|difficulty| difficulty.as_str()))
    .bind(create.effort_hint.map(|effort| effort.as_str()))
    .bind(create.routing_strategy.as_str())
    .bind(create.expected_context_size)
    .bind(create.result_rollup_policy.map(|policy| policy.as_str()))
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
    let rows = sqlx::query_as::<_, DocketJobRow>(&format!(
        r"
        SELECT {JOB_COLUMNS}
        FROM bear_jobs j
        WHERE j.bear_id = $1
          AND ($2::text IS NULL OR j.source_conversation_id = $2)
        ORDER BY j.updated_at DESC
        LIMIT $3
        ",
    ))
    .bind(bear_id)
    .bind(filter.source_conversation_id.as_deref())
    .bind(limit)
    .fetch_all(pool)
    .await?;

    // ponytail: list queries derive each row through the canonical projection, an O(n)
    // read pattern. Batch projection loading if list sizes make this material in production.
    let mut jobs = Vec::with_capacity(rows.len());
    for mut job in rows {
        if !filter.include_cancelled && job.lifecycle_intent.as_deref() == Some("cancelled") {
            continue;
        }
        if !filter.include_archived && job.lifecycle_intent.as_deref() == Some("archived") {
            continue;
        }
        let projection = get_job(pool, bear_id, job.id)
            .await?
            .expect("job selected from bear_jobs must remain present");
        job.status = derived_docket_job_status(&projection);
        if filter
            .statuses
            .as_ref()
            .is_some_and(|statuses| !statuses.iter().any(|status| job.status == status.as_str()))
        {
            continue;
        }
        jobs.push(job);
    }
    Ok(jobs)
}

pub(super) async fn get_job(
    pool: &PgPool,
    bear_id: Uuid,
    job_id: Uuid,
) -> Result<Option<DocketJobProjection>, DenError> {
    let Some(job) = sqlx::query_as::<_, DocketJobRow>(&format!(
        r"
        SELECT {JOB_COLUMNS}
        FROM bear_jobs j
        WHERE j.bear_id = $1 AND j.id = $2
        ",
    ))
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
               kind, scope, title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
               result_rollup_policy, created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
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

    let active_task_ids = list_active_task_ids(pool, job.id).await?;
    let mut projection = DocketJobProjection {
        job,
        current_run,
        criteria,
        criteria_states,
        tasks,
        task_states,
        active_task_ids,
    };
    projection.job.status = derived_docket_job_status(&projection);
    Ok(Some(projection))
}

async fn list_active_task_ids(pool: &PgPool, job_id: Uuid) -> Result<Vec<Uuid>, DenError> {
    sqlx::query_as::<_, (Uuid,)>(
        r"
        SELECT DISTINCT executing_task_id
        FROM bear_work_runs
        WHERE job_id = $1
          AND executing_task_id IS NOT NULL
          AND state IN ('queued', 'claimed', 'provisioning', 'running', 'paused', 'reporting')
        ",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await
    .map(|rows| rows.into_iter().map(|(task_id,)| task_id).collect())
    .map_err(Into::into)
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
    if update.status.is_some_and(|status| {
        matches!(
            status,
            DocketJobStatus::Ready
                | DocketJobStatus::Running
                | DocketJobStatus::Blocked
                | DocketJobStatus::Completed
        )
    }) {
        return Err(DenError::ValidationError(
            "Docket job ready/running/blocked/completed status is derived from current task and criterion state"
                .to_string(),
        ));
    }
    let mut tx = pool.begin().await?;
    let Some(current) = sqlx::query_as::<_, DocketJobRow>(&format!(
        r"
        SELECT {JOB_COLUMNS}
        FROM bear_jobs j
        WHERE j.bear_id = $1 AND j.id = $2
        ",
    ))
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
    let lifecycle_intent = update.status.and_then(|status| match status {
        DocketJobStatus::Cancelled => Some("cancelled"),
        DocketJobStatus::Archived => Some("archived"),
        _ => None,
    });
    sqlx::query(
        r"
        UPDATE bear_jobs
        SET goal = $3,
            commit_policy = $4,
            work_branch = $5,
            lifecycle_intent = COALESCE($6, lifecycle_intent),
            visibility = $7,
            updated_at = NOW()
        WHERE bear_id = $1 AND id = $2
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
            .commit_policy
            .map(|policy| policy.map(|policy| policy.as_str().to_string()))
            .unwrap_or_else(|| current.commit_policy.clone()),
    )
    .bind(
        update
            .work_branch
            .clone()
            .unwrap_or_else(|| current.work_branch.clone()),
    )
    .bind(lifecycle_intent)
    .bind(
        update
            .visibility
            .map(|visibility| visibility.as_str())
            .unwrap_or(&current.visibility),
    )
    .execute(&mut *tx)
    .await?;
    if let Some(work_surface_id) = update.work_surface_id {
        let work_surface_id = work_surface_id.ok_or_else(|| {
            DenError::ValidationError(
                "Docket work jobs cannot clear their required work surface".to_string(),
            )
        })?;
        sqlx::query("DELETE FROM job_work_surface_assignments WHERE job_id = $1")
            .bind(update.job_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO job_work_surface_assignments (job_id, work_surface_id, mutation_policy) VALUES ($1, $2, 'required')",
        )
        .bind(update.job_id)
        .bind(work_surface_id)
        .execute(&mut *tx)
        .await?;
    }
    let job = sqlx::query_as::<_, DocketJobRow>(&format!(
        "SELECT {JOB_COLUMNS} FROM bear_jobs j WHERE j.bear_id = $1 AND j.id = $2",
    ))
    .bind(update.bear_id)
    .bind(update.job_id)
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
        "lifecycle_intent": job.lifecycle_intent,
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
        Some(DocketJobStatus::Archived) => "job_archived",
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
        DocketJobStatus::Archived => (Some("cancelled"), true),
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

async fn reconcile_job_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job_id: Uuid,
    run_id: Uuid,
) -> Result<(), DenError> {
    let (_, current_run_id): (Option<String>, Option<Uuid>) = sqlx::query_as(
        "SELECT lifecycle_intent, current_run_id FROM bear_jobs WHERE id = $1 FOR UPDATE",
    )
    .bind(job_id)
    .fetch_one(&mut **tx)
    .await?;
    if current_run_id != Some(run_id) {
        return Ok(());
    }

    let (in_progress, blocked, unfinished): (i64, i64, i64) = sqlx::query_as(
        r"
        SELECT
            COUNT(*) FILTER (
                WHERE EXISTS (
                    SELECT 1 FROM bear_work_runs work_run
                    WHERE work_run.job_run_id = $2
                      AND work_run.executing_task_id = task.id
                      AND work_run.state IN ('claimed', 'provisioning', 'running', 'paused', 'reporting')
                )
            ),
            COUNT(*) FILTER (WHERE COALESCE(state.status, 'pending') = 'blocked'),
            COUNT(*) FILTER (WHERE COALESCE(state.status, 'pending') NOT IN ('done', 'cancelled'))
        FROM bear_tasks task
        LEFT JOIN bear_task_run_state state
          ON state.task_id = task.id AND state.run_id = $2
        WHERE task.job_id = $1
        ",
    )
    .bind(job_id)
    .bind(run_id)
    .fetch_one(&mut **tx)
    .await?;
    let unmet_criteria: i64 = sqlx::query_scalar(
        r"
        SELECT COUNT(*)
        FROM bear_job_criteria criterion
        LEFT JOIN bear_job_criteria_state state
          ON state.criterion_id = criterion.id AND state.run_id = $2
        WHERE criterion.job_id = $1
          AND COALESCE(state.status, 'unmet') NOT IN ('met', 'waived')
        ",
    )
    .bind(job_id)
    .bind(run_id)
    .fetch_one(&mut **tx)
    .await?;

    let _derived = derived_job_status(in_progress, blocked, unfinished, unmet_criteria);
    Ok(())
}

fn derived_job_status(
    in_progress: i64,
    blocked: i64,
    unfinished: i64,
    unmet_criteria: i64,
) -> &'static str {
    if in_progress > 0 {
        "running"
    } else if blocked > 0 {
        "blocked"
    } else if unfinished == 0 && unmet_criteria == 0 {
        "completed"
    } else {
        "ready"
    }
}

#[cfg(test)]
mod derived_job_status_tests {
    use super::derived_job_status;

    #[test]
    fn derives_status_from_current_work_only() {
        assert_eq!(derived_job_status(1, 1, 2, 1), "running");
        assert_eq!(derived_job_status(0, 1, 2, 0), "blocked");
        assert_eq!(derived_job_status(0, 0, 1, 0), "ready");
        assert_eq!(derived_job_status(0, 0, 0, 1), "ready");
        assert_eq!(derived_job_status(0, 0, 0, 0), "completed");
    }

    #[test]
    fn stale_task_progress_without_a_work_run_is_ready() {
        // The query supplying `in_progress` counts only task rows backed by an
        // active work run; a stale task state therefore reaches this branch.
        assert_eq!(derived_job_status(0, 0, 1, 0), "ready");
    }
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
    reconcile_job_status(&mut tx, update.job_id, update.run_id).await?;
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
    if let Some(source_conversation_id) = lookup.source_conversation_id {
        let row = sqlx::query_as::<_, DocketExecutionSessionRow>(SELECT_EXECUTION_BY_CONVERSATION)
            .bind(bear_id)
            .bind(owner_profile.as_str())
            .bind(source_conversation_id)
            .fetch_optional(pool)
            .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    if let Some(session_id) = lookup.session_id {
        let row = sqlx::query_as::<_, DocketExecutionSessionRow>(SELECT_EXECUTION_BY_SESSION)
            .bind(bear_id)
            .bind(owner_profile.as_str())
            .bind(session_id)
            .fetch_optional(pool)
            .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    if let Some(source_client_session_id) = lookup.source_client_session_id {
        return sqlx::query_as::<_, DocketExecutionSessionRow>(SELECT_EXECUTION_BY_ACP_SESSION)
            .bind(bear_id)
            .bind(owner_profile.as_str())
            .bind(source_client_session_id)
            .fetch_optional(pool)
            .await
            .map_err(Into::into);
    }
    Ok(None)
}

pub(super) async fn clear_active_execution_sessions(
    pool: &PgPool,
    bear_id: Uuid,
    lookup: DocketExecutionLookup,
) -> Result<u64, DenError> {
    if lookup.source_conversation_id.is_none()
        && lookup.session_id.is_none()
        && lookup.source_client_session_id.is_none()
    {
        return Err(DenError::ValidationError(
            "Docket execution clear requires a conversation, session, or client session id"
                .to_string(),
        ));
    }

    // ponytail: clear is lookup-based and marks matching active-like rows cancelled. The ceiling is
    // richer clear reasons/history; add a Docket event if operator-facing audit needs it.
    let result = sqlx::query(
        r"
        UPDATE docket_execution_sessions
        SET state = 'cancelled', updated_at = NOW()
        WHERE bear_id = $1
          AND state IN ('active', 'blocked', 'completing', 'paused')
          AND (
            ($2::TEXT IS NOT NULL AND source_conversation_id = $2)
            OR ($3::TEXT IS NOT NULL AND session_id = $3)
            OR ($4::TEXT IS NOT NULL AND source_client_session_id = $4)
          )
        ",
    )
    .bind(bear_id)
    .bind(lookup.source_conversation_id)
    .bind(lookup.session_id)
    .bind(lookup.source_client_session_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
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
    let state_by_task = projection
        .task_states
        .iter()
        .map(|state| (state.task_id, state.status.as_str()))
        .collect::<HashMap<_, _>>();
    let criteria_complete = projection.criteria.is_empty()
        || projection.criteria.iter().all(|criterion| {
            projection
                .criteria_states
                .iter()
                .find(|state| state.criterion_id == criterion.id)
                .map(|state| matches!(state.status.as_str(), "met" | "waived"))
                .unwrap_or(false)
        });
    let tasks_complete = projection.tasks.iter().all(|task| {
        matches!(
            state_by_task.get(&task.id).copied(),
            Some("done" | "cancelled")
        )
    });
    // A criteria-only block is re-evaluated here after criteria are updated.
    // A blocked run with unfinished task work still requires explicit recovery.
    if (projection.job.status == "blocked" || run.state == "blocked") && !tasks_complete {
        return Err(DenError::ValidationError(
            "Docket job is blocked; recover its current run before dispatching work".to_string(),
        ));
    }
    let active_task_ids = projection
        .active_task_ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    if let Some(active) = active_task_ids.iter().next() {
        // Re-evaluate the task attached to the live work run against the plan.
        // Execution is run-owned; task state records only durable outcomes.
        let selected =
            first_pending_leaf_in_plan_order(&projection, &state_by_task).map(|task| task.id);
        if selected != Some(*active) {
            return Err(DenError::ValidationError(format!(
                "Docket active task {active} is not the first eligible leaf in sibling order; refusing stale work run",
            )));
        }
        let job = get_job(pool, request.bear_id, request.job_id)
            .await?
            .ok_or_else(|| {
                DenError::NotFound(format!("Docket job not found: {}", request.job_id))
            })?;
        return Ok(DocketJobExecuteOutcome {
            job,
            selected_task_id: Some(*active),
            completed: false,
            blocked: false,
            message: "Job has the first eligible active task.".to_string(),
        });
    }

    if let Some(next) = first_pending_leaf_in_plan_order(&projection, &state_by_task) {
        mark_job_running(pool, &request, run.id).await?;
        record_execution_session(pool, &request, run.id, Some(next.id), "active").await?;
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

    if tasks_complete && criteria_complete {
        record_execution_session(pool, &request, run.id, None, "completed").await?;
        let job = get_job(pool, request.bear_id, request.job_id)
            .await?
            .ok_or_else(|| {
                DenError::NotFound(format!("Docket job not found: {}", request.job_id))
            })?;
        Ok(DocketJobExecuteOutcome {
            job,
            selected_task_id: None,
            completed: true,
            blocked: false,
            message: "All tasks and criteria are complete; job completed.".to_string(),
        })
    } else {
        record_execution_session(pool, &request, run.id, None, "blocked").await?;
        let job = get_job(pool, request.bear_id, request.job_id)
            .await?
            .ok_or_else(|| {
                DenError::NotFound(format!("Docket job not found: {}", request.job_id))
            })?;
        Ok(DocketJobExecuteOutcome {
            job,
            selected_task_id: None,
            completed: false,
            blocked: true,
            message:
                "No task is actionable, but required work or acceptance criteria remain incomplete."
                    .to_string(),
        })
    }
}

/// Returns the first unfinished leaf in depth-first sibling order.
///
/// A task with children is a phase/roll-up, not independently executable. A
/// leaf becomes eligible only after all its preceding siblings are terminal.
fn first_pending_leaf_in_plan_order<'a>(
    projection: &'a DocketJobProjection,
    state_by_task: &HashMap<Uuid, &str>,
) -> Option<&'a DocketTaskRow> {
    let children = projection.tasks.iter().fold(
        HashMap::<Option<Uuid>, Vec<&DocketTaskRow>>::new(),
        |mut children, task| {
            children.entry(task.parent_task_id).or_default().push(task);
            children
        },
    );
    let mut visited = HashSet::new();
    first_pending_leaf_in_children(None, &children, state_by_task, &mut visited)
        .ok()
        .flatten()
}

/// Returns the next pending leaf, or `Err` when earlier non-terminal work
/// blocks advancement to later siblings.
fn first_pending_leaf_in_children<'a>(
    parent_id: Option<Uuid>,
    children: &HashMap<Option<Uuid>, Vec<&'a DocketTaskRow>>,
    state_by_task: &HashMap<Uuid, &str>,
    visited: &mut HashSet<Uuid>,
) -> Result<Option<&'a DocketTaskRow>, ()> {
    let Some(siblings) = children.get(&parent_id) else {
        return Ok(None);
    };
    let mut siblings = siblings.clone();
    siblings.sort_by_key(|task| (task.sibling_order, task.created_at));

    for task in siblings {
        if !visited.insert(task.id) {
            continue;
        }
        if children.contains_key(&Some(task.id)) {
            match first_pending_leaf_in_children(Some(task.id), children, state_by_task, visited) {
                Ok(Some(next)) => return Ok(Some(next)),
                Err(()) => return Err(()),
                Ok(None) => continue,
            }
        }
        match state_by_task.get(&task.id).copied().unwrap_or("pending") {
            "done" | "cancelled" => {}
            "pending" => return Ok(Some(task)),
            // An earlier in-progress or blocked leaf owns its place in the
            // plan. Do not skip it to offer a later sibling.
            _ => return Err(()),
        }
    }
    Ok(None)
}

async fn mark_job_running(
    pool: &PgPool,
    request: &DocketJobExecuteRequest,
    run_id: Uuid,
) -> Result<(), DenError> {
    let mut tx = pool.begin().await?;
    // Job status is derived from run/task/criterion evidence. Starting this run
    // is the only status transition needed here.
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
                   kind, scope, title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
                   result_rollup_policy, created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id, settled_by_entry_id,
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

pub(super) async fn append_entry(
    pool: &PgPool,
    create: DocketEntryCreate,
) -> Result<DocketEntryRow, DenError> {
    let summary = create.summary.trim();
    if summary.is_empty() {
        return Err(DenError::ValidationError(
            "Docket entry summary must not be empty".to_string(),
        ));
    }
    if create.kind == DocketEntryKind::Outcome {
        return Err(DenError::ValidationError(
            "terminal outcomes are created by task settlement".to_string(),
        ));
    }
    if create.kind == DocketEntryKind::Question && create.actor_role != BearProfile::Pair {
        return Err(DenError::ValidationError(
            "Docket questions may only be recorded by pair".to_string(),
        ));
    }

    let mut tx = pool.begin().await?;
    let task_job_id = if let Some(task_id) = create.task_id {
        Some(
            sqlx::query_scalar::<_, Option<Uuid>>(
                "SELECT job_id FROM bear_tasks WHERE id = $1 AND bear_id = $2",
            )
            .bind(task_id)
            .bind(create.bear_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DenError::NotFound(format!("Docket task `{task_id}` not found")))?
            .ok_or_else(|| {
                DenError::ValidationError(
                    "Docket journal entries require a job-backed task".to_string(),
                )
            })?,
        )
    } else {
        None
    };
    let job_id = create.job_id.or(task_job_id).ok_or_else(|| {
        DenError::ValidationError("Docket entry requires job_id or task_id".to_string())
    })?;
    if task_job_id.is_some_and(|task_job_id| task_job_id != job_id) {
        return Err(DenError::ValidationError(
            "Docket entry task does not belong to job".to_string(),
        ));
    }
    let job_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM bear_jobs WHERE id = $1 AND bear_id = $2)",
    )
    .bind(job_id)
    .bind(create.bear_id)
    .fetch_one(&mut *tx)
    .await?;
    if !job_exists {
        return Err(DenError::NotFound(format!(
            "Docket job `{job_id}` not found"
        )));
    }
    match create.scope {
        DocketEntryScope::TaskJournal if create.task_id.is_none() => {
            return Err(DenError::ValidationError(
                "task journal entry requires task_id".to_string(),
            ));
        }
        DocketEntryScope::JobNotebook if create.job_id.is_none() => {
            return Err(DenError::ValidationError(
                "job notebook entry requires job_id".to_string(),
            ));
        }
        _ => {}
    }
    if let Some(run_id) = create.run_id {
        let run_matches = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM bear_job_runs WHERE id = $1 AND job_id = $2)",
        )
        .bind(run_id)
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
        if !run_matches {
            return Err(DenError::ValidationError(
                "Docket entry run does not belong to job".to_string(),
            ));
        }
    }

    let row = sqlx::query_as::<_, DocketEntryRow>(
        r"
        INSERT INTO bear_docket_entries (
            job_id, task_id, run_id, scope, kind, summary, body, evidence_refs,
            related_task_ids, tags, by_role, by_agent_id, by_user_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9::jsonb, $10::jsonb, $11, $12, $13)
        RETURNING id, job_id, task_id, run_id, scope, kind, summary, body,
                  disposition, evidence_refs, related_task_ids, tags, by_role,
                  by_agent_id, by_user_id, NULL::uuid AS source_entry_id, created_at
        ",
    )
    .bind(job_id)
    .bind(create.task_id)
    .bind(create.run_id)
    .bind(create.scope.as_str())
    .bind(create.kind.as_str())
    .bind(summary)
    .bind(
        create
            .body
            .as_deref()
            .map(str::trim)
            .filter(|body| !body.is_empty()),
    )
    .bind(Value::Array(create.evidence_refs))
    .bind(json!(create.related_task_ids))
    .bind(json!(create.tags))
    .bind(create.actor_role.as_str())
    .bind(create.actor_agent_id.as_deref())
    .bind(create.actor_user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub(super) async fn promote_entry(
    pool: &PgPool,
    promotion: DocketEntryPromotion,
) -> Result<DocketEntryRow, DenError> {
    let mut tx = pool.begin().await?;
    let source = sqlx::query_as::<_, DocketEntryRow>(
        r"
        SELECT e.id, e.job_id, e.task_id, e.run_id, e.scope, e.kind, e.summary,
               e.body, e.disposition, e.evidence_refs, e.related_task_ids, e.tags,
               e.by_role, e.by_agent_id, e.by_user_id, e.source_entry_id, e.created_at
        FROM bear_docket_entries e
        JOIN bear_jobs j ON j.id = e.job_id
        WHERE e.id = $1 AND j.bear_id = $2
        FOR UPDATE OF e
        ",
    )
    .bind(promotion.entry_id)
    .bind(promotion.bear_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        DenError::NotFound(format!("Docket entry `{}` not found", promotion.entry_id))
    })?;
    if source.scope != DocketEntryScope::TaskJournal.as_str()
        || source.kind == DocketEntryKind::Outcome.as_str()
        || source.source_entry_id.is_some()
    {
        return Err(DenError::ValidationError(
            "only non-outcome task journal entries may be promoted".to_string(),
        ));
    }

    let row = sqlx::query_as::<_, DocketEntryRow>(
        r"
        INSERT INTO bear_docket_entries (
            job_id, task_id, run_id, scope, kind, summary, body, evidence_refs,
            related_task_ids, tags, by_role, by_agent_id, by_user_id, source_entry_id
        )
        VALUES (
            $1, $2, $3, 'job_notebook', $4, $5, $6, $7, $8, $9, $10, $11, $12, $13
        )
        ON CONFLICT (source_entry_id) WHERE source_entry_id IS NOT NULL DO UPDATE
        SET source_entry_id = EXCLUDED.source_entry_id
        RETURNING id, job_id, task_id, run_id, scope, kind, summary, body,
                  disposition, evidence_refs, related_task_ids, tags, by_role,
                  by_agent_id, by_user_id, source_entry_id, created_at
        ",
    )
    .bind(source.job_id)
    .bind(source.task_id)
    .bind(source.run_id)
    .bind(source.kind)
    .bind(source.summary)
    .bind(source.body)
    .bind(source.evidence_refs)
    .bind(source.related_task_ids)
    .bind(source.tags)
    .bind(promotion.actor_role.as_str())
    .bind(promotion.actor_agent_id.as_deref())
    .bind(promotion.actor_user_id)
    .bind(source.id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub(super) async fn list_entries(
    pool: &PgPool,
    bear_id: Uuid,
    filter: DocketEntryListFilter,
) -> Result<Vec<DocketEntryRow>, DenError> {
    let limit = if filter.limit <= 0 {
        100
    } else {
        filter.limit.min(500)
    };
    sqlx::query_as::<_, DocketEntryRow>(
        r"
        SELECT e.id, e.job_id, e.task_id, e.run_id, e.scope, e.kind, e.summary,
               e.body, e.disposition, e.evidence_refs, e.related_task_ids, e.tags,
               e.by_role, e.by_agent_id, e.by_user_id, e.source_entry_id, e.created_at
        FROM bear_docket_entries e
        JOIN bear_jobs j ON j.id = e.job_id
        WHERE j.bear_id = $1
          AND ($2::uuid IS NULL OR e.job_id = $2)
          AND ($3::uuid IS NULL OR e.task_id = $3)
        ORDER BY e.created_at DESC, e.id DESC
        LIMIT $4
        ",
    )
    .bind(bear_id)
    .bind(filter.job_id)
    .bind(filter.task_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
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
                   kind, scope, title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
                   result_rollup_policy, created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id, settled_by_entry_id,
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
                   child.routing_strategy, child.expected_context_size, child.result_rollup_policy, child.created_by_role, child.created_by_user_id,
                   child.created_by_agent_id, child.created_in_run_id, child.settled_by_entry_id, child.created_at,
                   child.updated_at
            FROM bear_tasks child
            JOIN task_tree parent ON child.parent_task_id = parent.id
        )
        SELECT id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
               kind, scope, title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
               result_rollup_policy, created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
               settled_by_entry_id, created_at, updated_at
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
    validate_task_update_scope(&mut tx, &current, &update).await?;
    validate_in_progress_task_edit_is_paused(&mut tx, &current, &update).await?;
    if let Some(run_state) = update
        .run_state
        .as_ref()
        .filter(|state| state.status.as_str() == "done")
    {
        if has_primary_output_evidence(run_state.result_refs.as_ref()) {
            validate_primary_output_registry(&mut tx, &current, run_state).await?;
            record_completion_receipt(&mut tx, &current, run_state).await?;
        }
        validate_parent_completion(&mut tx, &current, run_state.run_id).await?;
    }
    let patched = update_task_definition(&mut tx, &current, &update.definition).await?;
    append_task_updated_events(&mut tx, &patched, &update).await?;
    let append_outcome = should_append_terminal_outcome(&mut tx, &patched, &update).await?;
    let run_state = if let Some(run_state) = update.run_state.as_ref() {
        Some(upsert_task_run_state(&mut tx, update.task_id, run_state).await?)
    } else {
        None
    };
    if append_outcome {
        append_terminal_outcome(&mut tx, &patched, &update).await?;
    }
    if let Some(run_state) = update
        .run_state
        .as_ref()
        .filter(|state| matches!(state.status.as_str(), "done" | "cancelled"))
    {
        roll_up_completed_parents(&mut tx, current.parent_task_id, run_state.run_id).await?;
    }
    if let (Some(job_id), Some(run_state)) = (current.job_id, update.run_state.as_ref()) {
        reconcile_job_status(&mut tx, job_id, run_state.run_id).await?;
    }
    tx.commit().await?;
    Ok(DocketTaskProjection {
        task: patched,
        run_state,
    })
}

pub(super) async fn settle_session_task(
    pool: &PgPool,
    settlement: DocketSessionTaskSettlement,
) -> Result<DocketTaskProjection, DenError> {
    let status = settlement.status.as_str();
    if !matches!(status, "done" | "blocked" | "cancelled") {
        return Err(DenError::ValidationError(
            "session task settlement requires a terminal status".to_string(),
        ));
    }
    let summary = settlement
        .result_summary
        .as_deref()
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .ok_or_else(|| {
            DenError::ValidationError(
                "Docket terminal task settlement requires non-empty result_summary".to_string(),
            )
        })?;
    let disposition = settlement
        .outcome_disposition
        .unwrap_or(match settlement.status {
            super::model::DocketTaskStatus::Done => {
                super::model::DocketOutcomeDisposition::Completed
            }
            super::model::DocketTaskStatus::Blocked => {
                super::model::DocketOutcomeDisposition::Blocked
            }
            super::model::DocketTaskStatus::Cancelled => {
                super::model::DocketOutcomeDisposition::Cancelled
            }
            super::model::DocketTaskStatus::Pending => unreachable!("validated terminal status"),
        });
    if !disposition.is_valid_for(settlement.status) {
        return Err(DenError::ValidationError(
            "Docket outcome disposition contradicts task status".to_string(),
        ));
    }
    let mut tx = pool.begin().await?;
    let task = select_task(&mut tx, settlement.bear_id, settlement.task_id).await?;
    if task.job_id.is_some() || task.session_anchor_id != Some(settlement.session_anchor_id) {
        return Err(DenError::ValidationError(
            "session task settlement requires a task owned by the current session".to_string(),
        ));
    }
    if task.settled_by_entry_id.is_some() {
        return Err(DenError::ValidationError(
            "Docket terminal settlement is append-only; reopen task before replacing its outcome"
                .to_string(),
        ));
    }
    let entry_id = sqlx::query_scalar::<_, Uuid>(
        r"INSERT INTO bear_docket_entries (job_id, task_id, run_id, scope, kind, summary, disposition, evidence_refs, by_role, by_agent_id, by_user_id)
           VALUES (NULL, $1, NULL, 'task_journal', 'outcome', $2, $3, $4::jsonb, $5, $6, $7)
           RETURNING id",
    )
    .bind(task.id)
    .bind(summary)
    .bind(disposition.as_str())
    .bind(terminal_evidence_refs(settlement.result_refs.as_ref()))
    .bind(settlement.actor_role.as_str())
    .bind(settlement.actor_agent_id.as_deref())
    .bind(settlement.actor_user_id)
    .fetch_one(&mut *tx)
    .await?;
    let task = sqlx::query_as::<_, DocketTaskRow>(
        "UPDATE bear_tasks SET settled_by_entry_id = $2, updated_at = NOW() WHERE id = $1 RETURNING id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order, kind, scope, title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size, result_rollup_policy, created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id, settled_by_entry_id, created_at, updated_at",
    )
    .bind(task.id)
    .bind(entry_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(DocketTaskProjection {
        task,
        run_state: None,
    })
}

async fn validate_primary_output_registry(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &DocketTaskRow,
    run_state: &super::model::DocketTaskRunStateUpdate,
) -> Result<(), DenError> {
    let result_refs = run_state
        .result_refs
        .as_ref()
        .expect("validated before transaction");
    let primary_output = result_refs["primary_output"]
        .as_object()
        .expect("validated before transaction");
    let artifact_ref = required_string(primary_output, "artifact_ref", "primary_output")?;
    let immutable_identity =
        required_string(primary_output, "immutable_identity", "primary_output")?;
    let kind = required_string(primary_output, "kind", "primary_output")?;
    match kind {
        "den_artifact" => {
            let artifact = sqlx::query(
                "SELECT content_sha256
                 FROM artifacts
                 JOIN artifact_links ON artifact_links.artifact_id = artifacts.id
                 WHERE artifacts.bear_id = $1
                   AND artifacts.artifact_ref = $2
                   AND artifacts.lifecycle = 'finalized'
                   AND artifacts.storage_kind IN ('db_text', 'garage_artifacts')
                   AND artifact_links.target_kind = 'docket_task'
                   AND artifact_links.target_id = $3
                   AND artifact_links.role = 'primary_output'",
            )
            .bind(task.bear_id)
            .bind(artifact_ref)
            .bind(task.id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
            let Some(artifact) = artifact else {
                return Err(DenError::ValidationError(
                    "Docket den_artifact primary_output must be finalized and linked to this task as primary_output".to_string(),
                ));
            };
            let content_sha256: Option<String> = artifact.try_get("content_sha256")?;
            if content_sha256.as_deref() != Some(immutable_identity) {
                return Err(DenError::ValidationError(
                    "Docket den_artifact primary_output immutable_identity must equal its finalized content SHA-256"
                        .to_string(),
                ));
            }
        }
        "git_commit" => {
            let artifact = sqlx::query(
                "SELECT metadata->'git'->>'commit_oid' AS commit_oid
                 FROM artifacts
                 JOIN artifact_links ON artifact_links.artifact_id = artifacts.id
                 WHERE artifacts.bear_id = $1
                   AND artifacts.artifact_ref = $2
                   AND artifacts.lifecycle = 'finalized'
                   AND artifacts.storage_kind = 'external_git_commit'
                   AND artifact_links.target_kind = 'docket_task'
                   AND artifact_links.target_id = $3
                   AND artifact_links.role = 'primary_output'",
            )
            .bind(task.bear_id)
            .bind(artifact_ref)
            .bind(task.id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
            let Some(artifact) = artifact else {
                return Err(DenError::ValidationError(
                    "Docket git_commit primary_output must be a finalized Git commit artifact linked to this task as primary_output".to_string(),
                ));
            };
            let commit_oid: Option<String> = artifact.try_get("commit_oid")?;
            if commit_oid.as_deref() != Some(immutable_identity) {
                return Err(DenError::ValidationError(
                    "Docket git_commit primary_output immutable_identity must equal its finalized commit OID"
                        .to_string(),
                ));
            }
        }
        _ => unreachable!("validated primary output kind"),
    }
    Ok(())
}

async fn record_completion_receipt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &DocketTaskRow,
    run_state: &super::model::DocketTaskRunStateUpdate,
) -> Result<(), DenError> {
    let result_refs = run_state
        .result_refs
        .as_ref()
        .expect("validated before transaction");
    let primary_output = result_refs["primary_output"]
        .as_object()
        .expect("validated before transaction");
    let validation = result_refs["validation"]
        .as_object()
        .expect("validated before transaction");
    let primary_output_ref = required_string(primary_output, "artifact_ref", "primary_output")?;
    let immutable_identity =
        required_string(primary_output, "immutable_identity", "primary_output")?;
    sqlx::query(
        "INSERT INTO docket_task_completion_receipts
             (task_id, run_id, primary_output_ref, immutable_identity, validation)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (task_id, run_id) DO UPDATE
         SET primary_output_ref = EXCLUDED.primary_output_ref,
             immutable_identity = EXCLUDED.immutable_identity,
             validation = EXCLUDED.validation,
             recorded_at = now()",
    )
    .bind(task.id)
    .bind(run_state.run_id)
    .bind(primary_output_ref)
    .bind(immutable_identity)
    .bind(Value::Object(validation.clone()))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn validate_parent_completion(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &DocketTaskRow,
    run_id: Uuid,
) -> Result<(), DenError> {
    let unfinished_children = sqlx::query_scalar::<_, i64>(
        r"
        WITH RECURSIVE descendants AS (
            SELECT id FROM bear_tasks WHERE parent_task_id = $1
            UNION ALL
            SELECT child.id
            FROM bear_tasks child
            JOIN descendants parent ON child.parent_task_id = parent.id
        )
        SELECT COUNT(*)
        FROM descendants
        LEFT JOIN bear_task_run_state state
          ON state.task_id = descendants.id AND state.run_id = $2
        WHERE COALESCE(state.status, 'pending') NOT IN ('done', 'cancelled')
        ",
    )
    .bind(task.id)
    .bind(run_id)
    .fetch_one(&mut **tx)
    .await?;

    if unfinished_children > 0 {
        return Err(DenError::ValidationError(format!(
            "Docket phase cannot be completed while {unfinished_children} child task(s) remain unfinished: task_id={}",
            task.id
        )));
    }
    Ok(())
}

async fn roll_up_completed_parents(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    mut parent_id: Option<Uuid>,
    run_id: Uuid,
) -> Result<(), DenError> {
    while let Some(task_id) = parent_id {
        let unfinished_descendants = sqlx::query_scalar::<_, i64>(
            r"
            WITH RECURSIVE descendants AS (
                SELECT id FROM bear_tasks WHERE parent_task_id = $1
                UNION ALL
                SELECT child.id
                FROM bear_tasks child
                JOIN descendants parent ON child.parent_task_id = parent.id
            )
            SELECT COUNT(*)
            FROM descendants
            LEFT JOIN bear_task_run_state state
              ON state.task_id = descendants.id AND state.run_id = $2
            WHERE COALESCE(state.status, 'pending') NOT IN ('done', 'cancelled')
            ",
        )
        .bind(task_id)
        .bind(run_id)
        .fetch_one(&mut **tx)
        .await?;
        if unfinished_descendants != 0 {
            break;
        }

        sqlx::query(
            r"
            UPDATE bear_task_run_state
            SET status = 'done',
                result_summary = COALESCE(NULLIF(result_summary, ''), 'All child tasks are terminal.'),
                finished_at = COALESCE(finished_at, NOW()),
                updated_at = NOW()
            WHERE run_id = $1 AND task_id = $2 AND status NOT IN ('done', 'cancelled')
            ",
        )
        .bind(run_id)
        .bind(task_id)
        .execute(&mut **tx)
        .await?;
        parent_id = sqlx::query_scalar("SELECT parent_task_id FROM bear_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&mut **tx)
            .await?;
    }
    Ok(())
}

fn validate_docket_task_run_state_update(
    update: Option<&super::model::DocketTaskRunStateUpdate>,
) -> Result<(), DenError> {
    let Some(update) = update else {
        return Ok(());
    };
    if !matches!(update.status.as_str(), "done" | "blocked" | "cancelled") {
        return Ok(());
    }
    if update
        .result_summary
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        return Err(DenError::ValidationError(
            "Docket terminal task settlement requires non-empty result_summary".to_string(),
        ));
    }
    validate_primary_output_evidence(update.result_refs.as_ref())
}

fn has_primary_output_evidence(result_refs: Option<&Value>) -> bool {
    result_refs
        .and_then(Value::as_object)
        .is_some_and(|refs| refs.contains_key("primary_output") || refs.contains_key("validation"))
}

pub(super) fn validate_primary_output_evidence(
    result_refs: Option<&Value>,
) -> Result<(), DenError> {
    if !has_primary_output_evidence(result_refs) {
        return Ok(());
    }
    let result_refs = result_refs
        .and_then(Value::as_object)
        .expect("primary_output evidence requires a result_refs object");
    let Some(primary_output) = result_refs.get("primary_output").and_then(Value::as_object) else {
        return Err(DenError::ValidationError(
            "Docket task completion requires a primary_output object".to_string(),
        ));
    };
    let primary_ref = required_string(primary_output, "artifact_ref", "primary_output")?;
    let primary_identity = required_string(primary_output, "immutable_identity", "primary_output")?;
    let kind = required_string(primary_output, "kind", "primary_output")?;
    if !matches!(kind, "git_commit" | "den_artifact") {
        return Err(DenError::ValidationError(
            "Docket primary_output kind must be git_commit or den_artifact".to_string(),
        ));
    }
    let Some(validation) = result_refs.get("validation").and_then(Value::as_object) else {
        return Err(DenError::ValidationError(
            "Docket task completion requires validation evidence".to_string(),
        ));
    };
    if required_string(validation, "primary_output_ref", "validation")? != primary_ref
        || required_string(validation, "immutable_identity", "validation")? != primary_identity
    {
        return Err(DenError::ValidationError(
            "Docket validation must reference the primary_output's immutable identity".to_string(),
        ));
    }
    let result = required_string(validation, "result", "validation")?;
    if result != "passed" {
        return Err(DenError::ValidationError(
            "Docket task completion requires passing validation evidence".to_string(),
        ));
    }
    required_string(validation, "command", "validation")?;
    required_string(validation, "execution_provenance", "validation")?;
    Ok(())
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a str, DenError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            DenError::ValidationError(format!("Docket {context} requires non-empty {field}"))
        })
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
               kind, scope, title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
               result_rollup_policy, created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
               created_at, updated_at
        FROM bear_tasks
        WHERE bear_id = $1 AND id = $2
        FOR UPDATE
        ",
    )
    .bind(bear_id)
    .bind(task_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        DenError::NotFound(format!(
            "Docket task definition not found in bear scope: task_id={task_id}, bear_id={bear_id}"
        ))
    })
}

async fn validate_task_update_scope(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    current: &DocketTaskRow,
    update: &DocketTaskUpdate,
) -> Result<(), DenError> {
    if let Some(job_id) = update.job_id {
        if current.job_id != Some(job_id) {
            return Err(DenError::ValidationError(format!(
                "Docket task belongs to a different job: task_id={}, expected_job_id={job_id}, actual_job_id={}",
                update.task_id,
                current
                    .job_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "none".to_string())
            )));
        }
        if let Some(run_state) = update.run_state.as_ref() {
            let run = sqlx::query_as::<_, DocketJobRunRow>(
                r"
                SELECT id, job_id, trigger, schedule_ref, state, started_at, finished_at,
                       outcome, created_at, updated_at
                FROM bear_job_runs
                WHERE job_id = $1 AND id = $2
                ",
            )
            .bind(job_id)
            .bind(run_state.run_id)
            .fetch_optional(&mut **tx)
            .await?;
            if run.is_none() {
                return Err(DenError::NotFound(format!(
                    "Docket task run state scope not found: task_id={}, job_id={job_id}, run_id={}",
                    update.task_id, run_state.run_id
                )));
            }
        }
    }
    Ok(())
}

async fn validate_in_progress_task_edit_is_paused(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    current: &DocketTaskRow,
    update: &DocketTaskUpdate,
) -> Result<(), DenError> {
    let Some(run_state) = update.run_state.as_ref() else {
        return Ok(());
    };
    let definition_changed = update.definition.title.is_some()
        || update.definition.body.is_some()
        || update.definition.completion_criteria.is_some()
        || update.definition.parent_task_id.is_some()
        || update.definition.sibling_order.is_some()
        || update.definition.kind.is_some()
        || update.definition.scope.is_some()
        || update.definition.difficulty.is_some()
        || update.definition.effort_hint.is_some()
        || update.definition.routing_strategy.is_some()
        || update.definition.expected_context_size.is_some()
        || update.definition.result_rollup_policy.is_some();
    if !definition_changed {
        return Ok(());
    }
    let active = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM bear_work_runs WHERE job_run_id=$1 AND executing_task_id=$2 AND state IN ('claimed', 'provisioning', 'running', 'paused', 'reporting'))",
    )
    .bind(run_state.run_id)
    .bind(current.id)
    .fetch_one(&mut **tx)
    .await?;
    if !active {
        return Ok(());
    }
    let paused = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM bear_work_runs WHERE job_run_id=$1 AND executing_task_id=$2 AND state='paused')",
    )
    .bind(run_state.run_id)
    .bind(current.id)
    .fetch_one(&mut **tx)
    .await?;
    if paused {
        Ok(())
    } else {
        Err(DenError::ValidationError(
            "editing an in-progress Docket task requires its job run to be paused".into(),
        ))
    }
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
            routing_strategy = $12,
            expected_context_size = $13,
            result_rollup_policy = $14,
            updated_at = NOW()
        WHERE bear_id = $1 AND id = $2
        RETURNING id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                  kind, scope, title, body, completion_criteria, difficulty, effort_hint, routing_strategy, expected_context_size,
                  result_rollup_policy, created_by_role, created_by_user_id, created_by_agent_id, created_in_run_id,
                  settled_by_entry_id, created_at, updated_at
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
            .routing_strategy
            .map(|strategy| strategy.as_str())
            .unwrap_or(&current.routing_strategy),
    )
    .bind(
        patch
            .expected_context_size
            .unwrap_or(current.expected_context_size),
    )
    .bind(
        patch
            .result_rollup_policy
            .map(|value| value.map(|policy| policy.as_str().to_string()))
            .unwrap_or_else(|| current.result_rollup_policy.clone()),
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
    sqlx::query_as::<_, DocketTaskRunStateRow>(
        r"
        INSERT INTO bear_task_run_state (
            run_id, task_id, status, result_refs, result_summary, started_at, finished_at, updated_at
        )
        VALUES (
            $1, $2, $3, $4::jsonb, $5,
            NULL,
            CASE WHEN $3 IN ('done', 'cancelled') THEN NOW() ELSE NULL END,
            NOW()
        )
        ON CONFLICT (run_id, task_id) DO UPDATE
        SET status = EXCLUDED.status,
            result_refs = EXCLUDED.result_refs,
            result_summary = EXCLUDED.result_summary,
            started_at = bear_task_run_state.started_at,
            finished_at = CASE
                WHEN EXCLUDED.status IN ('done', 'cancelled') THEN COALESCE(bear_task_run_state.finished_at, NOW())
                WHEN EXCLUDED.status IN ('pending', 'blocked') THEN NULL
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

async fn should_append_terminal_outcome(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &DocketTaskRow,
    update: &DocketTaskUpdate,
) -> Result<bool, DenError> {
    let Some(run_state) = update.run_state.as_ref() else {
        return Ok(false);
    };
    let Some(disposition) = terminal_outcome_disposition(run_state)? else {
        return Ok(false);
    };
    let previous_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM bear_task_run_state WHERE run_id = $1 AND task_id = $2",
    )
    .bind(run_state.run_id)
    .bind(task.id)
    .fetch_optional(&mut **tx)
    .await?;
    if previous_status.as_deref() != Some(run_state.status.as_str()) {
        return Ok(true);
    }

    let summary = run_state
        .result_summary
        .as_deref()
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .ok_or_else(|| {
            DenError::ValidationError(format!(
                "Docket terminal task settlement requires non-empty result_summary: status={}",
                run_state.status.as_str()
            ))
        })?;
    let evidence_refs = terminal_evidence_refs(run_state.result_refs.as_ref());
    let existing = sqlx::query(
        r"
        SELECT summary, disposition, evidence_refs
        FROM bear_docket_entries
        WHERE task_id = $1 AND run_id = $2 AND kind = 'outcome'
        ORDER BY created_at DESC, id DESC
        LIMIT 1
        ",
    )
    .bind(task.id)
    .bind(run_state.run_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(existing) = existing else {
        // ponytail: repair pre-journal terminal state on its next settlement retry;
        // remove this fallback once all pre-journal runs have aged out.
        return Ok(true);
    };
    let existing_summary: String = existing.try_get("summary")?;
    let existing_disposition: String = existing.try_get("disposition")?;
    let existing_evidence: Value = existing.try_get("evidence_refs")?;
    if existing_summary == summary
        && existing_disposition == disposition
        && existing_evidence == evidence_refs
    {
        return Ok(false);
    }
    Err(DenError::ValidationError(format!(
        "Docket terminal settlement is append-only; reopen task before replacing its outcome: task_id={}, run_id={}",
        task.id, run_state.run_id
    )))
}

fn terminal_outcome_disposition(
    run_state: &super::model::DocketTaskRunStateUpdate,
) -> Result<Option<&'static str>, DenError> {
    use super::model::{DocketOutcomeDisposition, DocketTaskStatus};

    let default = match run_state.status {
        DocketTaskStatus::Pending => return Ok(None),
        DocketTaskStatus::Done => DocketOutcomeDisposition::Completed,
        DocketTaskStatus::Blocked => DocketOutcomeDisposition::Blocked,
        DocketTaskStatus::Cancelled => DocketOutcomeDisposition::Cancelled,
    };
    let disposition = run_state.outcome_disposition.unwrap_or(default);
    if !disposition.is_valid_for(run_state.status) {
        return Err(DenError::ValidationError(format!(
            "Docket outcome disposition '{}' contradicts task status '{}'",
            disposition.as_str(),
            run_state.status.as_str()
        )));
    }
    Ok(Some(disposition.as_str()))
}

fn terminal_evidence_refs(result_refs: Option<&Value>) -> Value {
    result_refs
        .map(|refs| match refs {
            Value::Array(refs) => Value::Array(refs.clone()),
            refs => Value::Array(vec![refs.clone()]),
        })
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

async fn append_terminal_outcome(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &DocketTaskRow,
    update: &DocketTaskUpdate,
) -> Result<(), DenError> {
    let Some(run_state) = update.run_state.as_ref() else {
        return Ok(());
    };
    let Some(disposition) = terminal_outcome_disposition(run_state)? else {
        return Ok(());
    };
    let summary = run_state
        .result_summary
        .as_deref()
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .ok_or_else(|| {
            DenError::ValidationError(format!(
                "Docket terminal task settlement requires non-empty result_summary: status={}",
                run_state.status.as_str()
            ))
        })?;
    let evidence_refs = terminal_evidence_refs(run_state.result_refs.as_ref());

    let entry_id = sqlx::query_scalar::<_, Uuid>(
        r"
        INSERT INTO bear_docket_entries (
            job_id, task_id, run_id, scope, kind, summary, disposition,
            evidence_refs, by_role, by_agent_id, by_user_id
        )
        VALUES ($1, $2, $3, 'task_journal', 'outcome', $4, $5, $6::jsonb, $7, $8, $9)
        RETURNING id
        ",
    )
    .bind(task.job_id)
    .bind(task.id)
    .bind(run_state.run_id)
    .bind(summary)
    .bind(disposition)
    .bind(evidence_refs)
    .bind(update.actor_role.as_str())
    .bind(update.actor_agent_id.as_deref())
    .bind(update.actor_user_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("UPDATE bear_tasks SET settled_by_entry_id = $2, updated_at = NOW() WHERE id = $1")
        .bind(task.id)
        .bind(entry_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
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
                    job_id: Some(job_id),
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
                        outcome_disposition: None,
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
                    placement: Some(DocketTaskPlacement::Last),
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
                    routing_strategy: super::model::RoutingStrategy::Auto,
                    expected_context_size: None,
                    result_rollup_policy: None,
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
