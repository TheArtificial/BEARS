// ROUTES: When modifying routes in this file, update /src/web/ROUTES.md
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use axum_extra::extract::Form;
use axum_extra::routing::RouterExt;
use minijinja::context;
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use uuid::Uuid;
use validator::{Validate, ValidationError, ValidationErrors};

use crate::{
    auth_backend::AuthSession,
    errors::CustomError,
    web::{self, AppState},
    core::{
        user::db as user_db,
        web_policy,
    },
};
use den_runtime::{
    bears::{db as bears_db, db::BearParams, provision, BearProfileBinding, BearProfile},
    memory::{
            admin_inspect::bear_memory_admin_stats, BearMemoryAdminStats, MemoryStoreManager,
        },
};
use den_core::DenError;

use crate::web::bear_create_support::{
    admin_bear_edit_page_context, admin_bear_new_form_context,
    ensure_stored_model_in_options_for_handle, model_catalog_select_context,
    validate_default_model_for_catalog, AdminBearPromptForm, AdminNewBearForm, NewBearForm,
};

use super::bear_domains;

pub fn router() -> Router<AppState> {
    bear_domains::router().merge(Router::new())
        .route_with_tsr("/bears/", get(list_view))
        .route_with_tsr("/bears/new", get(new_view).post(new_action))
        .route_with_tsr("/bears/{id}/edit", get(edit_view).post(edit_action))
        .route_with_tsr(
            "/bears/{id}/edit/prompt",
            get(edit_prompt_view).post(edit_prompt_action),
        )
        .route_with_tsr("/bears/{id}/members/grant", post(grant_member_action))
        .route_with_tsr(
            "/bears/{id}/members/{user_id}/revoke",
            post(revoke_member_action),
        )
        .route_with_tsr("/bears/{id}/web-sources", post(add_web_source_action))
        .route_with_tsr(
            "/bears/{id}/web-sources/{source_id}/delete",
            post(delete_web_source_action),
        )
        .route_with_tsr("/bears/{id}/web-approvals", post(add_web_approval_action))
        .route_with_tsr(
            "/bears/{id}/web-approvals/{approval_id}/revoke",
            post(revoke_web_approval_action),
        )
        .route_with_tsr(
            "/bears/{id}/provision-missing-profiles",
            post(provision_missing_profiles_action),
        )
        .route_with_tsr(
            "/bears/{id}/provision-missing-roles",
            post(provision_missing_profiles_action),
        )
        .route_with_tsr("/bears/{id}/retry-letta", post(retry_letta_action))
        .route_with_tsr("/bears/{id}", get(detail_view))
}

#[derive(Debug, Serialize)]
pub(super) struct BearWebSourceRow {
    id: Uuid,
    scope_kind: String,
    scope_value: String,
    label: Option<String>,
    policy: String,
    priority: i32,
    created_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct BearWebApprovalRow {
    id: Uuid,
    scope_kind: String,
    scope_value: String,
    source: String,
    approved_by_user_label: Option<String>,
    created_at: String,
    expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct BearWebFetchRow {
    url: String,
    final_url: Option<String>,
    host: String,
    execution_location: String,
    approval_kind: String,
    http_status: Option<i32>,
    content_type: Option<String>,
    bytes: Option<i64>,
    fetched_at: String,
}

#[derive(Debug, Deserialize)]
struct AddWebSourceForm {
    scope_kind: String,
    scope_value: String,
    policy: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    priority: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct AddWebApprovalForm {
    scope_kind: String,
    scope_value: String,
}

#[derive(Debug, Serialize)]
pub(super) struct BearPlanModeRow {
    id: Uuid,
    user_id: i32,
    username: Option<String>,
    acp_session_id: String,
    state: String,
    reason: String,
    plan_artifact_path: Option<String>,
    plan_title: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct BearProfileBindingHealthRow {
    profile: String,
    binding_id: String,
    runtime_family: String,
    branch: String,
    letta_agent_id: Option<String>,
    provisioning_status: String,
    last_provisioned_version: i32,
    last_synced_at: Option<String>,
    health_status: String,
    health_label: String,
    health_detail: Option<String>,
    letta_name: Option<String>,
    letta_model: Option<String>,
    letta_agent_type: Option<String>,
    letta_tool_count: Option<usize>,
    letta_memory_block_count: Option<usize>,
    memfs_view_state: Option<String>,
    memfs_view_quarantined: bool,
    memfs_view_diagnostic: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct BearMemberAdminRow {
    pub(super) user_id: i32,
    pub(super) username: String,
    pub(super) display_name: String,
    pub(super) role: Option<String>,
    pub(super) role_label: String,
}

pub(super) fn membership_role_label(role: Option<&str>) -> String {
    match role.map(str::trim).filter(|s| !s.is_empty()) {
        Some("admin") => "Admin — can manage bear settings and members".to_string(),
        Some("member") | None => "Member — can use the bear".to_string(),
        Some(other) => format!("Custom ({other})"),
    }
}

impl BearProfileBindingHealthRow {
    fn native(agent: &BearProfileBinding, role: BearProfile) -> Self {
        let binding = agent
            .letta_agent_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let (health_status, health_label, health_detail) = match agent.provisioning_status.as_str()
        {
            "ready" => ("ok", "Ready", None),
            "failed" => (
                "error",
                "Failed",
                agent.last_provisioning_error.clone(),
            ),
            "drifted" => (
                "error",
                "Drifted",
                Some("Runtime config changed; re-provision to refresh.".to_string()),
            ),
            other => ("unknown", other, agent.last_provisioning_error.clone()),
        };
        Self {
            profile: role.as_str().to_string(),
            binding_id: agent.binding_id.clone(),
            runtime_family: role.runtime_family().to_string(),
            branch: role.as_str().to_string(),
            letta_agent_id: binding,
            provisioning_status: agent.provisioning_status.clone(),
            last_provisioned_version: agent.last_provisioned_version,
            last_synced_at: agent.last_synced_at.map(|t| t.to_string()),
            health_status: health_status.to_string(),
            health_label: health_label.to_string(),
            health_detail,
            letta_name: None,
            letta_model: None,
            letta_agent_type: None,
            letta_tool_count: None,
            letta_memory_block_count: None,
            memfs_view_state: None,
            memfs_view_quarantined: false,
            memfs_view_diagnostic: None,
        }
    }

}

pub(super) async fn bear_web_sources(
    pool: &sqlx::PgPool,
    bear_id: Uuid,
) -> Result<Vec<BearWebSourceRow>, CustomError> {
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            String,
            i32,
            time::OffsetDateTime,
        ),
    >(
        r#"
        SELECT id, scope_kind, scope_value, label, policy, priority, created_at
        FROM bear_web_sources
        WHERE bear_id = $1
        ORDER BY policy ASC, priority DESC, scope_kind ASC, scope_value ASC
        "#,
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, scope_kind, scope_value, label, policy, priority, created_at)| BearWebSourceRow {
                id,
                scope_kind,
                scope_value,
                label,
                policy,
                priority,
                created_at: created_at.to_string(),
            },
        )
        .collect())
}

pub(super) async fn bear_web_approvals(
    pool: &sqlx::PgPool,
    bear_id: Uuid,
) -> Result<Vec<BearWebApprovalRow>, CustomError> {
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            time::OffsetDateTime,
            Option<time::OffsetDateTime>,
        ),
    >(
        r#"
        SELECT a.id,
               a.scope_kind,
               a.scope_value,
               a.source,
               u.username,
               NULLIF(u.display_name, '') AS display_name,
               a.created_at,
               a.expires_at
        FROM bear_web_approvals a
        LEFT JOIN users u ON u.id = a.approved_by_user_id
        WHERE a.bear_id = $1 AND a.revoked_at IS NULL
        ORDER BY a.created_at DESC
        "#,
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                scope_kind,
                scope_value,
                source,
                username,
                display_name,
                created_at,
                expires_at,
            )| BearWebApprovalRow {
                id,
                scope_kind,
                scope_value,
                source,
                approved_by_user_label: match (display_name, username) {
                    (Some(display_name), Some(username)) => {
                        Some(format!("{display_name} (@{username})"))
                    }
                    (Some(display_name), None) => Some(display_name),
                    (None, Some(username)) => Some(format!("@{username}")),
                    (None, None) => None,
                },
                created_at: created_at.to_string(),
                expires_at: expires_at.map(|t| t.to_string()),
            },
        )
        .collect())
}

pub(super) async fn bear_web_fetches(
    pool: &sqlx::PgPool,
    bear_id: Uuid,
) -> Result<Vec<BearWebFetchRow>, CustomError> {
    let rows = sqlx::query_as::<_, (String, Option<String>, String, String, String, Option<i32>, Option<String>, Option<i64>, time::OffsetDateTime)>(
        r#"
        SELECT url, final_url, host, execution_location, approval_kind, http_status, content_type, bytes, fetched_at
        FROM bear_web_fetches
        WHERE bear_id = $1
        ORDER BY fetched_at DESC
        LIMIT 25
        "#,
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                url,
                final_url,
                host,
                execution_location,
                approval_kind,
                http_status,
                content_type,
                bytes,
                fetched_at,
            )| BearWebFetchRow {
                url,
                final_url,
                host,
                execution_location,
                approval_kind,
                http_status,
                content_type,
                bytes,
                fetched_at: fetched_at.to_string(),
            },
        )
        .collect())
}

pub(super) async fn bear_plan_mode_rows(
    pool: &sqlx::PgPool,
    bear_id: Uuid,
) -> Result<Vec<BearPlanModeRow>, CustomError> {
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            i32,
            Option<String>,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            time::OffsetDateTime,
            time::OffsetDateTime,
        ),
    >(
        r#"
        SELECT p.id, p.user_id, u.username, p.acp_session_id, p.state, p.reason,
               p.plan_artifact_path, p.plan_title, p.created_at, p.updated_at
        FROM acp_plan_mode_sessions p
        LEFT JOIN users u ON u.id = p.user_id
        WHERE p.bear_id = $1
        ORDER BY p.updated_at DESC
        LIMIT 10
        "#,
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                user_id,
                username,
                acp_session_id,
                state,
                reason,
                plan_artifact_path,
                plan_title,
                created_at,
                updated_at,
            )| BearPlanModeRow {
                id,
                user_id,
                username,
                acp_session_id,
                state,
                reason,
                plan_artifact_path,
                plan_title,
                created_at: created_at.to_string(),
                updated_at: updated_at.to_string(),
            },
        )
        .collect())
}

pub(super) async fn bear_agent_health_rows(
    state: &AppState,
    bear_id: Uuid,
    _letta_configured: bool,
) -> Result<Vec<BearProfileBindingHealthRow>, CustomError> {
    bears_db::ensure_bear_profile_binding_rows(state.sqlx_pool(), bear_id).await?;
    let agents = bears_db::list_bear_profile_bindings(state.sqlx_pool(), bear_id).await?;
    Ok(agents
        .into_iter()
        .map(|agent| {
            let role = agent.parsed_profile().unwrap_or(BearProfile::Chat);
            BearProfileBindingHealthRow::native(&agent, role)
        })
        .collect())
}

async fn bear_detail_response(
    state: &AppState,
    auth_session: AuthSession,
    id: Uuid,
    message: Option<String>,
) -> Result<Response, CustomError> {
    let bear = bears_db::get_bear(state.sqlx_pool(), id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;

    let member_count = bears_db::count_bear_members(state.sqlx_pool(), id).await?;
    let native_runtime = true;
    let letta_configured = false;
    let agent_health_rows = bear_agent_health_rows(state, id, letta_configured).await?;
    let roles_ready = agent_health_rows
        .iter()
        .filter(|row| row.health_status == "ok")
        .count();
    let roles_error = agent_health_rows
        .iter()
        .filter(|row| row.health_status == "error")
        .count();

    let memory_stats: Option<BearMemoryAdminStats> = {
        let manager = MemoryStoreManager::new(state.config.as_ref());
        match bear_memory_admin_stats(&manager, state.config.as_ref(), id).await {
            Ok(stats) => Some(stats),
            Err(err) => {
                tracing::warn!(%id, "admin hub memory stats unavailable: {err}");
                None
            }
        }
    };

    let conversation_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM conversations WHERE bear_id = $1",
    )
    .bind(id)
    .fetch_one(state.sqlx_pool())
    .await
    .map_err(|err| CustomError::Database(format!("count bear conversations: {err}")))?;

    web::render_template(
        state,
        "admin/bears/hub.html",
        auth_session,
        context! {
            bear,
            message,
            member_count,
            native_runtime,
            context_profile_enabled => bear.context_profile.is_some(),
            letta_configured,
            agent_health_rows,
            roles_ready,
            roles_error,
            memory_stats,
            conversation_count,
            bear_nav_active => "hub",
        },
    )
    .await
}

#[derive(Debug, Deserialize)]
struct BearDetailQuery {
    #[serde(default)]
    message: Option<String>,
}

async fn detail_view(
    Path(id): Path<Uuid>,
    Query(query): Query<BearDetailQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    bear_detail_response(&state, auth_session, id, query.message).await
}

async fn list_view(
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bears = bears_db::list_bears(state.sqlx_pool()).await?;
    web::render_template(
        &state,
        "admin/bears/list.html",
        auth_session,
        context! { bears, native_runtime => true },
    )
    .await
}

async fn new_view(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Query(_query): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, CustomError> {
    let form = AdminNewBearForm::default();
    let users = user_db::get_users(state.sqlx_pool()).await?;
    let page = admin_bear_new_form_context(&state, &form.bear).await;
    web::render_template(
        &state,
        "admin/bears/new.html",
        auth_session,
        context! {
            form => form.bear,
            admin_form => form,
            users,
            ..page
        },
    )
    .await
}

pub async fn new_action(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(admin_form): Form<AdminNewBearForm>,
) -> Result<Response, CustomError> {
    let form = admin_form.bear.clone();
    let catalog_fetch = {
        let (configured, options, _) = model_catalog_select_context(&state).await;
        if !configured {
            None
        } else {
            let model_trim = form.default_model.trim();
            let h = (!model_trim.is_empty()).then_some(model_trim);
            Some(Ok(ensure_stored_model_in_options_for_handle(h, options)))
        }
    };

    let mut validation_errors = ValidationErrors::new();
    if let Err(e) = form.validate() {
        validation_errors = e;
    }

    let default_model_trim = form.default_model.trim();
    validate_default_model_for_catalog(&catalog_fetch, default_model_trim, &mut validation_errors);
    if default_model_trim.is_empty() && catalog_fetch.is_none() {
        validation_errors.add(
            "default_model",
            ValidationError::new("Default model is required."),
        );
    }

    let default_model_opt = if default_model_trim.is_empty() {
        None
    } else {
        Some(default_model_trim)
    };

    if bears_db::bear_slug_exists(state.sqlx_pool(), form.slug.trim()).await? {
        validation_errors.add(
            "slug",
            ValidationError::new("A bear with this slug already exists."),
        );
    }

    if validation_errors.is_empty() {
        let id = bears_db::create_bear(
            state.sqlx_pool(),
            BearParams {
                slug: form.slug.trim(),
                name: form.name.trim(),
                description: form.description.trim(),
                system_prompt: form.system_prompt.trim(),
                default_model: default_model_opt,
                tools_enabled: None::<Json<serde_json::Value>>,
                letta_agent_type: None,
                letta_tool_ids: Json(Vec::new()),
                context_profile: None,
            },
        )
        .await?;

        if let Err(e) = provision::provision_bear_if_configured(
            state.sqlx_pool(),
            state.config.as_ref(),
            id,
        )
        .await
        {
            tracing::warn!(%id, "Bear provision failed: {e}");
            let users = user_db::get_users(state.sqlx_pool()).await?;
            let page = admin_bear_new_form_context(&state, &form).await;
            return web::render_template(
                &state,
                "admin/bears/new.html",
                auth_session,
                context! {
                    form => form,
                    admin_form => admin_form,
                    users,
                    provision_error => e.to_string(),
                    ..page
                },
            )
            .await;
        }

        if let Ok(user_id) = admin_form.grant_user_id.trim().parse::<i32>() {
            if user_id > 0
                && user_db::get_user_by_id(state.sqlx_pool(), user_id)
                    .await?
                    .is_some()
            {
                let role = admin_form.grant_role.trim();
                let role_opt = match role {
                    "" | "member" => Some(bears_db::BEAR_ROLE_MEMBER),
                    "admin" => Some(bears_db::BEAR_ROLE_ADMIN),
                    other => Some(other),
                };
                bears_db::grant_membership(state.sqlx_pool(), user_id, id, role_opt).await?;
            }
        }

        Ok(Redirect::to(&format!("/admin/bears/{id}")).into_response())
    } else {
        let users = user_db::get_users(state.sqlx_pool()).await?;
        let page = admin_bear_new_form_context(&state, &form).await;
        web::render_template(
            &state,
            "admin/bears/new.html",
            auth_session,
            context! {
                errors => validation_errors,
                form => form,
                admin_form => admin_form,
                users,
                ..page
            },
        )
        .await
    }
}

async fn edit_view(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = bears_db::get_bear(state.sqlx_pool(), id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    let form = NewBearForm::from(&bear);
    let page = admin_bear_edit_page_context(&state, &form).await;
    web::render_template(
        &state,
        "admin/bears/edit.html",
        auth_session,
        context! {
            bear,
            form,
            context_profile_enabled => bear.context_profile.is_some(),
            ..page
        },
    )
    .await
}

async fn edit_action(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<NewBearForm>,
) -> Result<Response, CustomError> {
    let bear = bears_db::get_bear(state.sqlx_pool(), id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;

    let model_fetch = {
        let (configured, options, _) = model_catalog_select_context(&state).await;
        if !configured {
            None
        } else {
            let model_trim = form.default_model.trim();
            let h = (!model_trim.is_empty()).then_some(model_trim);
            Some(Ok(ensure_stored_model_in_options_for_handle(h, options)))
        }
    };

    let mut validation_errors = ValidationErrors::new();
    if let Err(e) = form.validate() {
        validation_errors = e;
    }

    let letta_agent_type_db: Option<String> = bear.letta_agent_type.clone();

    let default_model_trim = form.default_model.trim();
    validate_default_model_for_catalog(&model_fetch, default_model_trim, &mut validation_errors);

    let default_model_opt = if default_model_trim.is_empty() {
        None
    } else {
        Some(default_model_trim)
    };

    if bears_db::bear_slug_exists_excluding(state.sqlx_pool(), form.slug.trim(), id).await? {
        validation_errors.add(
            "slug",
            ValidationError::new("A bear with this slug already exists."),
        );
    }

    if validation_errors.is_empty() {
        let system_prompt = if bear.context_profile.is_some() {
            bear.system_prompt.as_str()
        } else {
            form.system_prompt.trim()
        };
        bears_db::update_bear(
            state.sqlx_pool(),
            id,
            BearParams {
                slug: form.slug.trim(),
                name: form.name.trim(),
                description: form.description.trim(),
                system_prompt,
                default_model: default_model_opt,
                tools_enabled: None::<Json<serde_json::Value>>,
                letta_agent_type: letta_agent_type_db.as_deref(),
                letta_tool_ids: Json(bear.letta_tool_ids.0.clone()),
                context_profile: bear.context_profile.clone(),
            },
        )
        .await?;

        if let Err(e) = provision::provision_bear_if_configured(
            state.sqlx_pool(),
            state.config.as_ref(),
            id,
        )
        .await
        {
            tracing::warn!(%id, "Native profile refresh after bear edit failed: {e}");
            let bear = bears_db::get_bear(state.sqlx_pool(), id)
                .await?
                .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
            let page = admin_bear_edit_page_context(&state, &form).await;
            return web::render_template(
                &state,
                "admin/bears/edit.html",
                auth_session,
                context! {
                    errors => ValidationErrors::new(),
                    form => form,
                    bear,
                    provision_error => e.to_string(),
                    ..page
                },
            )
            .await;
        }

        Ok(Redirect::to(&format!("/admin/bears/{id}")).into_response())
    } else {
        let page = admin_bear_edit_page_context(&state, &form).await;
        web::render_template(
            &state,
            "admin/bears/edit.html",
            auth_session,
            context! {
                errors => validation_errors,
                form => form,
                bear,
                ..page
            },
        )
        .await
    }
}

#[derive(Debug, Deserialize)]
struct GrantMemberForm {
    user_id: i32,
    role: String,
}

async fn edit_prompt_view(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = bears_db::get_bear(state.sqlx_pool(), id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    let (form, context_profile_enabled) = AdminBearPromptForm::from_bear(&bear)?;
    web::render_template(
        &state,
        "admin/bears/edit_prompt.html",
        auth_session,
        context! {
            bear,
            form,
            context_profile_enabled,
            native_runtime => true,
        },
    )
    .await
}

async fn edit_prompt_action(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<AdminBearPromptForm>,
) -> Result<Response, CustomError> {
    let bear = bears_db::get_bear(state.sqlx_pool(), id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    let context_profile_enabled = bear.context_profile.is_some();

    let mut validation_errors = ValidationErrors::new();
    if let Err(e) = form.validate() {
        validation_errors = e;
    }

    let context_profile = match form.context_profile_for_bear(&bear, context_profile_enabled) {
        Ok(profile) => profile,
        Err(CustomError::ValidationError(_)) => {
            validation_errors.add(
                "system_prompt",
                ValidationError::new("Check role prompts and shared context fields."),
            );
            None
        }
        Err(err) => return Err(err),
    };

    let system_prompt = match form.resolved_system_prompt(bear.name.trim(), &context_profile) {
        Ok(prompt) => prompt,
        Err(CustomError::ValidationError(_)) => {
            validation_errors.add(
                "system_prompt",
                ValidationError::new("System prompt is required."),
            );
            String::new()
        }
        Err(err) => return Err(err),
    };

    if validation_errors.is_empty() {
        bears_db::update_bear(
            state.sqlx_pool(),
            id,
            BearParams {
                slug: bear.slug.as_str(),
                name: bear.name.as_str(),
                description: bear.description.as_str(),
                system_prompt: system_prompt.trim(),
                default_model: bear.default_model.as_deref(),
                tools_enabled: None::<Json<serde_json::Value>>,
                letta_agent_type: bear.letta_agent_type.as_deref(),
                letta_tool_ids: Json(bear.letta_tool_ids.0.clone()),
                context_profile: context_profile.clone(),
            },
        )
        .await?;

        provision::provision_bear_if_configured(
            state.sqlx_pool(),
            state.config.as_ref(),
            id,
        )
        .await?;

        Ok(Redirect::to(&format!("/admin/bears/{id}")).into_response())
    } else {
        web::render_template(
            &state,
            "admin/bears/edit_prompt.html",
            auth_session,
            context! {
                errors => validation_errors,
                form => form,
                bear,
                context_profile_enabled,
                native_runtime => true,
            },
        )
        .await
    }
}

async fn grant_member_action(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    _auth_session: AuthSession,
    Form(form): Form<GrantMemberForm>,
) -> Result<Response, CustomError> {
    if bears_db::get_bear(state.sqlx_pool(), id).await?.is_none() {
        return Err(CustomError::NotFound("bear not found".to_string()));
    }
    if user_db::get_user_by_id(state.sqlx_pool(), form.user_id)
        .await?
        .is_none()
    {
        return Ok(Redirect::to(&format!(
            "/admin/bears/{id}/access?message={}",
            urlencoding::encode("User not found.")
        ))
        .into_response());
    }
    let role = form.role.trim();
    let role_opt = match role {
        "" | "member" => Some(bears_db::BEAR_ROLE_MEMBER),
        "admin" => Some(bears_db::BEAR_ROLE_ADMIN),
        other => Some(other),
    };
    bears_db::grant_membership(state.sqlx_pool(), form.user_id, id, role_opt).await?;
    Ok(Redirect::to(&format!(
        "/admin/bears/{id}/access?message={}",
        urlencoding::encode("Access granted.")
    ))
    .into_response())
}

async fn revoke_member_action(
    Path((id, user_id)): Path<(Uuid, i32)>,
    State(state): State<AppState>,
    _auth_session: AuthSession,
) -> Result<Response, CustomError> {
    match bears_db::revoke_membership(state.sqlx_pool(), user_id, id).await {
        Ok(()) => Ok(Redirect::to(&format!(
            "/admin/bears/{id}/access?message={}",
            urlencoding::encode("Access removed.")
        ))
        .into_response()),
        Err(DenError::NotFound(_)) => Ok(Redirect::to(&format!(
            "/admin/bears/{id}/access?message={}",
            urlencoding::encode("Membership not found.")
        ))
        .into_response()),
        Err(err) => Err(err.into()),
    }
}

async fn add_web_source_action(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    _auth_session: AuthSession,
    Form(form): Form<AddWebSourceForm>,
) -> Result<Response, CustomError> {
    let scope_kind = form.scope_kind.trim();
    let policy = form.policy.trim();
    if !matches!(scope_kind, "host" | "url")
        || !matches!(policy, "preferred" | "allowed" | "blocked")
    {
        return Ok(Redirect::to(&format!(
            "/admin/bears/{id}/policy?message={}",
            urlencoding::encode("Invalid web source policy form.")
        ))
        .into_response());
    }
    let scope_value = match web_policy::normalize_web_scope_value(scope_kind, &form.scope_value) {
        Ok(scope_value) => scope_value,
        Err(err) => {
            return Ok(Redirect::to(&format!(
                "/admin/bears/{id}/policy?message={}",
                urlencoding::encode(&err.to_string())
            ))
            .into_response());
        }
    };
    sqlx::query(
        r#"
        INSERT INTO bear_web_sources (bear_id, scope_kind, scope_value, label, policy, priority)
        VALUES ($1, $2, $3, NULLIF($4, ''), $5, $6)
        ON CONFLICT (bear_id, scope_kind, scope_value)
        DO UPDATE SET label = EXCLUDED.label,
                      policy = EXCLUDED.policy,
                      priority = EXCLUDED.priority,
                      updated_at = now()
        "#,
    )
    .bind(id)
    .bind(scope_kind)
    .bind(scope_value)
    .bind(form.label.trim())
    .bind(policy)
    .bind(form.priority.unwrap_or(0))
    .execute(state.sqlx_pool())
    .await?;
    Ok(Redirect::to(&format!(
        "/admin/bears/{id}/policy?message={}",
        urlencoding::encode("Web source saved.")
    ))
    .into_response())
}

async fn delete_web_source_action(
    Path((id, source_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
) -> Result<Response, CustomError> {
    sqlx::query("DELETE FROM bear_web_sources WHERE bear_id = $1 AND id = $2")
        .bind(id)
        .bind(source_id)
        .execute(state.sqlx_pool())
        .await?;
    Ok(Redirect::to(&format!(
        "/admin/bears/{id}/policy?message={}",
        urlencoding::encode("Web source deleted.")
    ))
    .into_response())
}

async fn add_web_approval_action(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    _auth_session: AuthSession,
    Form(form): Form<AddWebApprovalForm>,
) -> Result<Response, CustomError> {
    let scope_kind = form.scope_kind.trim();
    if !matches!(scope_kind, "host" | "url") {
        return Ok(Redirect::to(&format!(
            "/admin/bears/{id}/policy?message={}",
            urlencoding::encode("Invalid web approval scope.")
        ))
        .into_response());
    }
    let scope_value = match web_policy::normalize_web_scope_value(scope_kind, &form.scope_value) {
        Ok(scope_value) => scope_value,
        Err(err) => {
            return Ok(Redirect::to(&format!(
                "/admin/bears/{id}/policy?message={}",
                urlencoding::encode(&err.to_string())
            ))
            .into_response());
        }
    };
    web_policy::record_web_approval(
        state.sqlx_pool(),
        id,
        scope_kind,
        &scope_value,
        None,
        "admin",
        None,
    )
    .await?;
    Ok(Redirect::to(&format!(
        "/admin/bears/{id}/policy?message={}",
        urlencoding::encode("Web approval added.")
    ))
    .into_response())
}

async fn revoke_web_approval_action(
    Path((id, approval_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
) -> Result<Response, CustomError> {
    sqlx::query("UPDATE bear_web_approvals SET revoked_at = now() WHERE bear_id = $1 AND id = $2")
        .bind(id)
        .bind(approval_id)
        .execute(state.sqlx_pool())
        .await?;
    Ok(Redirect::to(&format!(
        "/admin/bears/{id}/policy?message={}",
        urlencoding::encode("Web approval revoked.")
    ))
    .into_response())
}

async fn provision_missing_profiles_action(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    _auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let message = match provision::provision_missing_bear_profiles(
        state.sqlx_pool(),
        state.config.as_ref(),
        id,
    )
    .await
    {
        Ok(0) => "No missing native profile bindings to provision.".to_string(),
        Ok(n) => format!("Provisioned {n} missing native profile binding(s)."),
        Err(err) => format!("Provisioning native profile bindings failed: {err}"),
    };

    Ok(Redirect::to(&format!(
        "/admin/bears/{id}/roles?message={}",
        urlencoding::encode(&message)
    ))
    .into_response())
}

async fn retry_letta_action(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    _auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let message = match provision::reconcile_bear_native(
        state.sqlx_pool(),
        state.config.as_ref(),
        id,
    )
    .await
    {
        Ok(summary) => format!(
            "Native profile binding reconcile finished. {} profile(s) synced.",
            summary.synced_count()
        ),
        Err(err) => format!("Native profile binding reconcile failed: {err}"),
    };
    Ok(Redirect::to(&format!(
        "/admin/bears/{id}/advanced?message={}",
        urlencoding::encode(&message)
    ))
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use minijinja::Environment;
    use sqlx::{postgres::PgPoolOptions, types::Json};
    use std::sync::Arc;
    use tower::ServiceExt;
    use tower_sessions_sqlx_store::PostgresStore;

    use crate::{config::Config, startup::run_sqlx_migrations, web::AppState};

    static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn test_pool() -> Option<sqlx::PgPool> {
        dotenvy::dotenv().ok();
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("skipping DB-backed admin route test: DATABASE_URL is not set");
            return None;
        };
        let pool = match PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(&url)
            .await
        {
            Ok(pool) => pool,
            Err(err) => {
                eprintln!(
                    "skipping DB-backed admin route test: could not connect to DATABASE_URL: {err}"
                );
                return None;
            }
        };
        if let Err(err) = run_sqlx_migrations(&pool).await {
            eprintln!("skipping DB-backed admin route test: migrations failed: {err}");
            return None;
        }
        Some(pool)
    }

    fn test_state(pool: sqlx::PgPool) -> AppState {
        let config = Arc::new(Config::test_stub());
        let mut template_env = Environment::new();
        template_env
            .add_template("admin/bears/detail.html", "{{ web_message }} {{ web_sources | length }} {{ web_approvals | length }} {{ web_fetches | length }}{% for approval in web_approvals %} {{ approval.approved_by_user_label }}{% endfor %}")
            .expect("add test template");
        AppState::test_with_template_env(pool, template_env, config)
    }

    async fn test_app(pool: sqlx::PgPool) -> axum::Router {
        let store = PostgresStore::new(pool.clone());
        store.migrate().await.expect("session store migration");
        Router::new()
            .merge(router())
            .with_state(test_state(pool.clone()))
            .layer(
                axum_login::AuthManagerLayerBuilder::new(
                    crate::auth_backend::Backend::new(pool),
                    axum_login::tower_sessions::SessionManagerLayer::new(store),
                )
                .build(),
            )
    }

    async fn create_test_bear(pool: &sqlx::PgPool) -> Uuid {
        bears_db::create_bear(
            pool,
            BearParams {
                slug: &format!("web-admin-{}", Uuid::new_v4()),
                name: "Web Admin Test Bear",
                description: "",
                system_prompt: "System prompt",
                default_model: None,
                tools_enabled: None::<Json<serde_json::Value>>,
                letta_agent_type: None,
                letta_tool_ids: Json(Vec::new()),
                context_profile: None,
            },
        )
        .await
        .expect("create bear")
    }

    async fn create_test_user(pool: &sqlx::PgPool) -> i32 {
        sqlx::query_scalar::<_, i32>(
            r#"
            INSERT INTO users (email, username, display_name, passhash, is_admin)
            VALUES ($1, $2, $3, $4, true)
            RETURNING id
            "#,
        )
        .bind(format!("web-admin-{}@example.test", Uuid::new_v4()))
        .bind(format!("wa{}", &Uuid::new_v4().simple().to_string()[..28]))
        .bind("Admin Display")
        .bind("test-passhash")
        .fetch_one(pool)
        .await
        .expect("create user")
    }

    #[tokio::test]
    async fn add_web_source_route_normalizes_host_and_flashes() {
        let _guard = TEST_DB_LOCK.lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = create_test_bear(&pool).await;
        let _user_id = create_test_user(&pool).await;
        let app = test_app(pool.clone()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/bears/{bear_id}/web-sources"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("scope_kind=host&scope_value=Example.COM%3A8443.&policy=preferred&label=Docs&priority=10"))
                    .unwrap(),
            )
            .await
            .expect("add source response");

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert!(response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .contains("message=Web%20source%20saved"));
        let stored: String = sqlx::query_scalar(
            "SELECT scope_value FROM bear_web_sources WHERE bear_id = $1 AND scope_kind = 'host'",
        )
        .bind(bear_id)
        .fetch_one(&pool)
        .await
        .expect("stored source");
        assert_eq!(stored, "example.com:8443");
    }

    #[tokio::test]
    async fn add_web_source_route_rejects_url_in_host_scope() {
        let _guard = TEST_DB_LOCK.lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = create_test_bear(&pool).await;
        let _user_id = create_test_user(&pool).await;
        let app = test_app(pool.clone()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/bears/{bear_id}/web-sources"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("scope_kind=host&scope_value=https%3A%2F%2Fexample.com%2Fdocs&policy=preferred&label=&priority=0"))
                    .unwrap(),
            )
            .await
            .expect("validation response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("host must be a bare hostname"));
    }

    #[tokio::test]
    async fn add_and_revoke_web_approval_routes_update_active_approvals() {
        let _guard = TEST_DB_LOCK.lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = create_test_bear(&pool).await;
        let _user_id = create_test_user(&pool).await;
        let app = test_app(pool.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/bears/{bear_id}/web-approvals"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("scope_kind=host&scope_value=Docs.RS"))
                    .unwrap(),
            )
            .await
            .expect("add approval response");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let approval_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM bear_web_approvals WHERE bear_id = $1 AND scope_value = 'docs.rs' AND revoked_at IS NULL",
        )
        .bind(bear_id)
        .fetch_one(&pool)
        .await
        .expect("active approval");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/bears/{bear_id}/web-approvals/{approval_id}/revoke"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("revoke response");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let active_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::bigint FROM bear_web_approvals WHERE bear_id = $1 AND revoked_at IS NULL",
        )
        .bind(bear_id)
        .fetch_one(&pool)
        .await
        .expect("approval count");
        assert_eq!(active_count, 0);
    }

    #[tokio::test]
    async fn detail_route_displays_approval_user_label_and_recent_fetches() {
        let _guard = TEST_DB_LOCK.lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let bear_id = create_test_bear(&pool).await;
        let user_id = create_test_user(&pool).await;
        web_policy::record_web_approval(
            &pool,
            bear_id,
            "host",
            "example.com",
            Some(user_id),
            "admin",
            None,
        )
        .await
        .expect("record approval");
        web_policy::record_web_fetch_attempt(
            &pool,
            web_policy::WebFetchAuditParams {
                bear_id,
                session_id: Some("session-1"),
                tool_call_id: Some("tool-1"),
                url: "https://example.com/",
                final_url: None,
                host: "example.com",
                execution_location: "den",
                approval_kind: "user_host",
                http_status: Some(200),
                content_type: Some("text/html"),
                bytes: Some(123),
            },
        )
        .await
        .expect("record fetch");

        let app = test_app(pool.clone()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/bears/{bear_id}?message=Saved"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("detail response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("Saved"));
        assert!(body.contains("Admin Display"));
        assert!(body.contains("1 1") || body.contains("1 1 1"));
    }
}
