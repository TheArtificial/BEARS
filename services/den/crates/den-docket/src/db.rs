//! Docket Postgres access — internal to the module.
//!
//! These functions are the persistence layer behind `DocketService`; callers
//! outside `core::docket` go through the service, never here. Storage is still
//! the legacy `bear_work_plans` + `bear_work_plan_events` tables (pre-ADR-0034).

use std::collections::HashMap;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

use super::model::{
    validate_docket_job_create, validate_docket_task_create, validate_work_plan_update,
    BearWorkPlanRow, DocketJobCreate, DocketJobCriterionRow, DocketJobListFilter,
    DocketJobProjection, DocketJobRow, DocketJobRunRow, DocketTaskCreate, DocketTaskInput,
    DocketTaskRow, DocketTaskRunStateRow, WorkPlanListFilter, WorkPlanLookup, WorkPlanProjection,
    WorkPlanStatus, WorkPlanUpsert,
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
            bear_id, created_by_user_id, created_by_role, goal, work_surface_ref,
            commit_policy, status, visibility
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref,
                  commit_policy, status, visibility, current_run_id, created_at, updated_at
        ",
    )
    .bind(create.bear_id)
    .bind(create.created_by_user_id)
    .bind(create.created_by_role.trim())
    .bind(create.goal.trim())
    .bind(create.work_surface_ref.as_deref())
    .bind(create.commit_policy.map(|policy| policy.as_str()))
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
        RETURNING id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref,
                  commit_policy, status, visibility, current_run_id, created_at, updated_at
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
        if let Some(key) = task.client_key.as_ref().map(|key| key.trim()).filter(|key| !key.is_empty()) {
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
    Ok(DocketJobProjection {
        job,
        current_run: Some(run),
        criteria,
        tasks,
        task_states,
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
            difficulty, effort_hint, assigned_to_role, created_by_role, created_by_user_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        RETURNING id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                  kind, scope, title, body, difficulty, effort_hint, assigned_to_role,
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
        "parent_task_id": row.parent_task_id,
        "scope": row.scope,
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
        "title": row.title,
        "parent_task_id": row.parent_task_id,
        "scope": row.scope,
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
            title, body, difficulty, effort_hint, assigned_to_role, created_by_role,
            created_by_user_id, created_by_agent_id, created_in_run_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
        RETURNING id, bear_id, job_id, session_anchor_id, parent_task_id, sibling_order,
                  kind, scope, title, body, difficulty, effort_hint, assigned_to_role,
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
    let limit = if filter.limit <= 0 { 50 } else { filter.limit.min(200) };
    let rows = sqlx::query_as::<_, DocketJobRow>(
        r"
        SELECT id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref,
               commit_policy, status, visibility, current_run_id, created_at, updated_at
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
        SELECT id, bear_id, created_by_user_id, created_by_role, goal, work_surface_ref,
               commit_policy, status, visibility, current_run_id, created_at, updated_at
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
               kind, scope, title, body, difficulty, effort_hint, assigned_to_role,
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

    let task_states = if let Some(run) = current_run.as_ref() {
        list_task_run_states(pool, run.id).await?
    } else {
        Vec::new()
    };

    Ok(Some(DocketJobProjection {
        job,
        current_run,
        criteria,
        tasks,
        task_states,
    }))
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

pub(super) async fn create_or_update_work_plan(
    pool: &PgPool,
    params: WorkPlanUpsert,
) -> Result<BearWorkPlanRow, DenError> {
    validate_work_plan_update(&params.update)?;
    if !super::model::role_can_update_work_plan(params.owner_profile) {
        return Err(DenError::Authorization(format!(
            "the `{}` role cannot update work plans",
            params.owner_profile
        )));
    }

    let mut tx = pool.begin().await?;
    let existing_id = if let Some(plan_id) = params.plan_id {
        Some(plan_id)
    } else {
        find_existing_plan_id(
            &mut tx,
            params.bear_id,
            params.owner_profile,
            params.source_conversation_id.as_deref(),
            params.source_acp_session_id.as_deref(),
        )
        .await?
    };

    let (row, event_type) = if let Some(plan_id) = existing_id {
        let row = update_existing_plan(&mut tx, plan_id, &params).await?;
        (row, "updated")
    } else {
        let row = insert_new_plan(&mut tx, &params).await?;
        (row, "created")
    };

    append_event(
        &mut tx,
        WorkPlanEventParams {
            plan_id: row.id,
            bear_id: row.bear_id,
            actor_role: Some(params.owner_profile),
            actor_agent_id: params.owner_agent_id.as_deref(),
            actor_user_id: params.created_by_user_id,
            event_type,
            event_payload: json!({
                "version": row.version,
                "status": row.status,
                "visibility": row.visibility,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(row)
}

async fn find_existing_plan_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    bear_id: Uuid,
    owner_profile: BearProfile,
    source_conversation_id: Option<&str>,
    source_acp_session_id: Option<&str>,
) -> Result<Option<Uuid>, DenError> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        r"
        SELECT id
        FROM bear_work_plans
        WHERE bear_id = $1
          AND owner_profile = $2
          AND COALESCE(source_conversation_id, '') = COALESCE($3, '')
          AND COALESCE(source_acp_session_id, '') = COALESCE($4, '')
          AND status <> 'archived'
        ORDER BY updated_at DESC
        LIMIT 1
        ",
    )
    .bind(bear_id)
    .bind(owner_profile.as_str())
    .bind(source_conversation_id)
    .bind(source_acp_session_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|r| r.0))
}

async fn insert_new_plan(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    params: &WorkPlanUpsert,
) -> Result<BearWorkPlanRow, DenError> {
    sqlx::query_as::<_, BearWorkPlanRow>(
        r"
        INSERT INTO bear_work_plans (
            bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
            source_conversation_id, source_acp_session_id, source_channel, workspace_context,
            visibility, status, items
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb, $10::jsonb, $11, $12, $13::jsonb)
        RETURNING id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
                  source_conversation_id, source_acp_session_id, source_channel, workspace_context,
                  visibility, status, items, version, handoff_intent_path, handoff_task_id,
                  archived_at, created_at, updated_at
        ",
    )
    .bind(params.bear_id)
    .bind(params.update.title.trim())
    .bind(params.update.summary.trim())
    .bind(params.owner_profile.as_str())
    .bind(params.owner_agent_id.as_deref())
    .bind(params.created_by_user_id)
    .bind(params.source_conversation_id.as_deref())
    .bind(params.source_acp_session_id.as_deref())
    .bind(&params.source_channel)
    .bind(&params.update.workspace_context)
    .bind(params.update.visibility.as_str())
    .bind(params.update.status.as_str())
    .bind(serde_json::to_value(&params.update.items)?)
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

async fn update_existing_plan(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan_id: Uuid,
    params: &WorkPlanUpsert,
) -> Result<BearWorkPlanRow, DenError> {
    let row = sqlx::query_as::<_, BearWorkPlanRow>(
        r"
        UPDATE bear_work_plans
        SET title = $4,
            summary = $5,
            visibility = $6,
            status = $7,
            items = $8::jsonb,
            workspace_context = $9::jsonb,
            owner_agent_id = COALESCE($10, owner_agent_id),
            version = version + 1,
            archived_at = CASE WHEN $7 = 'archived' THEN COALESCE(archived_at, NOW()) ELSE archived_at END,
            updated_at = NOW()
        WHERE id = $1
          AND bear_id = $2
          AND owner_profile = $3
          AND ($11::integer IS NULL OR version = $11)
        RETURNING id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
                  source_conversation_id, source_acp_session_id, source_channel, workspace_context,
                  visibility, status, items, version, handoff_intent_path, handoff_task_id,
                  archived_at, created_at, updated_at
        ",
    )
    .bind(plan_id)
    .bind(params.bear_id)
    .bind(params.owner_profile.as_str())
    .bind(params.update.title.trim())
    .bind(params.update.summary.trim())
    .bind(params.update.visibility.as_str())
    .bind(params.update.status.as_str())
    .bind(serde_json::to_value(&params.update.items)?)
    .bind(&params.update.workspace_context)
    .bind(params.owner_agent_id.as_deref())
    .bind(params.expected_version)
    .fetch_optional(&mut **tx)
    .await?;
    row.ok_or_else(|| {
        DenError::ValidationError(
            "work plan was not found, is owned by another role, or version did not match"
                .to_string(),
        )
    })
}

struct WorkPlanEventParams<'a> {
    plan_id: Uuid,
    bear_id: Uuid,
    actor_role: Option<BearProfile>,
    actor_agent_id: Option<&'a str>,
    actor_user_id: Option<i32>,
    event_type: &'a str,
    event_payload: Value,
}

async fn append_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    params: WorkPlanEventParams<'_>,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        INSERT INTO bear_work_plan_events (
            plan_id, bear_id, actor_role, actor_agent_id, actor_user_id, event_type, event_payload
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)
        ",
    )
    .bind(params.plan_id)
    .bind(params.bear_id)
    .bind(params.actor_role.map(|role| role.as_str()))
    .bind(params.actor_agent_id)
    .bind(params.actor_user_id)
    .bind(params.event_type)
    .bind(params.event_payload)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn list_visible_work_plans(
    pool: &PgPool,
    bear_id: Uuid,
    viewer_role: BearProfile,
    user_id: i32,
    filter: WorkPlanListFilter,
) -> Result<Vec<WorkPlanProjection>, DenError> {
    let rows = sqlx::query_as::<_, BearWorkPlanRow>(
        r"
        SELECT id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
               source_conversation_id, source_acp_session_id, source_channel, workspace_context,
               visibility, status, items, version, handoff_intent_path, handoff_task_id,
               archived_at, created_at, updated_at
        FROM bear_work_plans
        WHERE bear_id = $1
        ORDER BY updated_at DESC
        LIMIT 50
        ",
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await?;

    let mut visible = Vec::new();
    for row in rows {
        if !filter.include_archived && row.status == WorkPlanStatus::Archived.as_str() {
            continue;
        }
        if let Some(owner_profile) = filter.owner_profile {
            if row.owner_profile != owner_profile.as_str() {
                continue;
            }
        }
        if let Some(statuses) = filter.statuses.as_ref() {
            if !statuses.iter().any(|status| row.status == status.as_str()) {
                continue;
            }
        }
        if let Some(projected) = row.project_for_profile(viewer_role, user_id)? {
            visible.push(projected);
        }
    }
    Ok(visible)
}

pub(super) async fn get_visible_work_plan(
    pool: &PgPool,
    bear_id: Uuid,
    viewer_role: BearProfile,
    user_id: i32,
    lookup: WorkPlanLookup,
) -> Result<Option<WorkPlanProjection>, DenError> {
    let row = if let Some(plan_id) = lookup.plan_id {
        sqlx::query_as::<_, BearWorkPlanRow>(SELECT_WORK_PLAN_BY_ID)
            .bind(bear_id)
            .bind(plan_id)
            .fetch_optional(pool)
            .await?
    } else if let Some(source_acp_session_id) = lookup.source_acp_session_id {
        sqlx::query_as::<_, BearWorkPlanRow>(SELECT_WORK_PLAN_BY_ACP_SESSION)
            .bind(bear_id)
            .bind(source_acp_session_id)
            .fetch_optional(pool)
            .await?
    } else if let Some(source_conversation_id) = lookup.source_conversation_id {
        sqlx::query_as::<_, BearWorkPlanRow>(SELECT_WORK_PLAN_BY_CONVERSATION)
            .bind(bear_id)
            .bind(source_conversation_id)
            .fetch_optional(pool)
            .await?
    } else {
        None
    };

    match row {
        Some(row) => row.project_for_profile(viewer_role, user_id),
        None => Ok(None),
    }
}

const SELECT_WORK_PLAN_BY_ID: &str = r"
    SELECT id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
           source_conversation_id, source_acp_session_id, source_channel, workspace_context,
           visibility, status, items, version, handoff_intent_path, handoff_task_id,
           archived_at, created_at, updated_at
    FROM bear_work_plans
    WHERE bear_id = $1 AND id = $2
";

const SELECT_WORK_PLAN_BY_ACP_SESSION: &str = r"
    SELECT id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
           source_conversation_id, source_acp_session_id, source_channel, workspace_context,
           visibility, status, items, version, handoff_intent_path, handoff_task_id,
           archived_at, created_at, updated_at
    FROM bear_work_plans
    WHERE bear_id = $1 AND source_acp_session_id = $2
    ORDER BY updated_at DESC
    LIMIT 1
";

const SELECT_WORK_PLAN_BY_CONVERSATION: &str = r"
    SELECT id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
           source_conversation_id, source_acp_session_id, source_channel, workspace_context,
           visibility, status, items, version, handoff_intent_path, handoff_task_id,
           archived_at, created_at, updated_at
    FROM bear_work_plans
    WHERE bear_id = $1 AND source_conversation_id = $2
    ORDER BY updated_at DESC
    LIMIT 1
";
