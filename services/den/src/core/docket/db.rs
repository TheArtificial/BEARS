//! Docket Postgres access — internal to the module.
//!
//! These functions are the persistence layer behind `DocketService`; callers
//! outside `core::docket` go through the service, never here. Storage is still
//! the legacy `bear_work_plans` + `bear_work_plan_events` tables (pre-ADR-0034).

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::{BearProfile, DenError};

use super::model::{
    validate_work_plan_update, BearWorkPlanRow, WorkPlanListFilter, WorkPlanLookup,
    WorkPlanProjection, WorkPlanStatus, WorkPlanUpsert,
};

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
        r#"
        SELECT id
        FROM bear_work_plans
        WHERE bear_id = $1
          AND owner_profile = $2
          AND COALESCE(source_conversation_id, '') = COALESCE($3, '')
          AND COALESCE(source_acp_session_id, '') = COALESCE($4, '')
          AND status <> 'archived'
        ORDER BY updated_at DESC
        LIMIT 1
        "#,
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
        r#"
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
        "#,
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
        r#"
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
        "#,
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
        r#"
        INSERT INTO bear_work_plan_events (
            plan_id, bear_id, actor_role, actor_agent_id, actor_user_id, event_type, event_payload
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)
        "#,
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
        r#"
        SELECT id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
               source_conversation_id, source_acp_session_id, source_channel, workspace_context,
               visibility, status, items, version, handoff_intent_path, handoff_task_id,
               archived_at, created_at, updated_at
        FROM bear_work_plans
        WHERE bear_id = $1
        ORDER BY updated_at DESC
        LIMIT 50
        "#,
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

const SELECT_WORK_PLAN_BY_ID: &str = r#"
    SELECT id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
           source_conversation_id, source_acp_session_id, source_channel, workspace_context,
           visibility, status, items, version, handoff_intent_path, handoff_task_id,
           archived_at, created_at, updated_at
    FROM bear_work_plans
    WHERE bear_id = $1 AND id = $2
"#;

const SELECT_WORK_PLAN_BY_ACP_SESSION: &str = r#"
    SELECT id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
           source_conversation_id, source_acp_session_id, source_channel, workspace_context,
           visibility, status, items, version, handoff_intent_path, handoff_task_id,
           archived_at, created_at, updated_at
    FROM bear_work_plans
    WHERE bear_id = $1 AND source_acp_session_id = $2
    ORDER BY updated_at DESC
    LIMIT 1
"#;

const SELECT_WORK_PLAN_BY_CONVERSATION: &str = r#"
    SELECT id, bear_id, title, summary, owner_profile, owner_agent_id, created_by_user_id,
           source_conversation_id, source_acp_session_id, source_channel, workspace_context,
           visibility, status, items, version, handoff_intent_path, handoff_task_id,
           archived_at, created_at, updated_at
    FROM bear_work_plans
    WHERE bear_id = $1 AND source_conversation_id = $2
    ORDER BY updated_at DESC
    LIMIT 1
"#;
