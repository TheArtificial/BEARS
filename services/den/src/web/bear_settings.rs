//! Bear-scoped settings at `/bear/{slug}/…` for members (read) and bear admins (write).

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
use uuid::Uuid;

use crate::{
    auth_backend::{AuthSession, SessionUser},
    core::{user::db as user_db, web_policy},
    errors::CustomError,
    web::{self, AppState},
};
use den_core::DenError;
use den_runtime::{
    bears::{
        context_profile_from_json, db as bears_db,
        db::{role_is_bear_admin, BEAR_ROLE_ADMIN, BEAR_ROLE_MEMBER},
        get_compiled_bear_config, list_bear_block_bindings, managed_blocks::BearCompiledConfigRow,
        provision, BearProfile,
    },
    conversation_persistence::{self, list_messages_page},
    memory::{
        admin_inspect::{
            bear_memory_admin_stats, get_memory_record_by_id, list_all_logical_paths,
            list_recent_memory_records,
        },
        store::list_memory_proposals, tools as sqlite_memory, BearMemoryAdminStats,
        MemoryStoreManager,
    },
    memory_proposals::{self, CreateMemoryProposal},
    pair_reflection,
    prompt_memory_block_store::list_prompt_memory_blocks_for_bear_profile,
};

use crate::web::admin::bears::{
    bear_agent_health_rows, bear_plan_mode_rows, bear_web_approvals, bear_web_fetches,
    bear_web_sources, membership_role_label, AddWebApprovalForm, AddWebSourceForm,
    BearMemberAdminRow, BearPlanModeRow, BearProfileBindingHealthRow, BearWebApprovalRow,
    BearWebFetchRow, BearWebSourceRow,
};

use super::{
    bear_member::{email_verify_redirect, load_bear_member, viewer_can_manage_bear},
    bear_profile::{build_role_detail_view, role_memory_label},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route_with_tsr("/bear/{slug}/overview", get(overview_view))
        .route_with_tsr("/bear/{slug}/access", get(access_view))
        .route_with_tsr("/bear/{slug}/persona", get(persona_view))
        .route_with_tsr("/bear/{slug}/profiles", get(profiles_view))
        .route_with_tsr("/bear/{slug}/profiles/{profile}", get(profile_detail_view))
        .route_with_tsr(
            "/bear/{slug}/memory",
            get(memory_view).post(memory_delete_post),
        )
        .route_with_tsr(
            "/bear/{slug}/memory/proposals/{proposal_id}",
            get(memory_proposal_get).post(memory_proposal_post),
        )
        .route_with_tsr(
            "/bear/{slug}/memory/records/{memory_id}",
            get(memory_record_view),
        )
        .route_with_tsr("/bear/{slug}/conversations", get(conversations_view))
        .route_with_tsr(
            "/bear/{slug}/conversations/{conversation_id}",
            get(conversation_detail_view),
        )
        .route_with_tsr("/bear/{slug}/context", get(context_view))
        .route_with_tsr("/bear/{slug}/policy", get(policy_view))
        .route_with_tsr("/bear/{slug}/advanced", get(advanced_view))
        .route_with_tsr("/bear/{slug}/members/grant", post(grant_member_action))
        .route_with_tsr(
            "/bear/{slug}/members/{user_id}/revoke",
            post(revoke_member_action),
        )
        .route_with_tsr("/bear/{slug}/web-sources", post(add_web_source_action))
        .route_with_tsr(
            "/bear/{slug}/web-sources/{source_id}/delete",
            post(delete_web_source_action),
        )
        .route_with_tsr("/bear/{slug}/web-approvals", post(add_web_approval_action))
        .route_with_tsr(
            "/bear/{slug}/web-approvals/{approval_id}/revoke",
            post(revoke_web_approval_action),
        )
        .route_with_tsr(
            "/bear/{slug}/provision-missing-roles",
            post(provision_missing_roles_action),
        )
}

#[derive(Debug, Deserialize)]
struct DomainQuery {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MemoryQuery {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    deleted: Option<usize>,
    #[serde(default)]
    review_requested: Option<usize>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MemoryDeleteForm {
    role: String,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    confirm: String,
    #[serde(default)]
    review_title: Option<String>,
    #[serde(default)]
    review_summary: Option<String>,
    #[serde(default)]
    review_rationale: Option<String>,
    #[serde(default)]
    suggested_action: Option<String>,
    #[serde(default)]
    sensitivity: Option<String>,
    #[serde(default)]
    requires_human: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MemoryProposalResolutionForm {
    status: String,
    #[serde(default)]
    review_notes: Option<String>,
    #[serde(default)]
    decision_summary: Option<String>,
}

#[derive(Serialize)]
struct MemoryRoleRow {
    profile: String,
    label: String,
    description: String,
    runtime_family: String,
    selected: bool,
    status_label: String,
    file_count: usize,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MemberGrantForm {
    #[serde(default)]
    username: String,
    #[serde(default)]
    user_id: Option<i32>,
    #[serde(default)]
    role: String,
}

#[derive(Debug, Serialize)]
struct ConversationAdminRow {
    id: Uuid,
    external_id: String,
    title: String,
    source_session: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct MessageAdminRow {
    sequence_no: i64,
    message_type: String,
    role: String,
    visibility: String,
    preview: String,
}

#[derive(Debug, Serialize)]
struct PromptMemoryAdminRow {
    block_id: String,
    scope: String,
    block_type: String,
    state: String,
    title: String,
    body_preview: String,
}

#[derive(Debug, Serialize)]
struct CompiledRolePromptRow {
    role: String,
    prompt_preview: String,
    char_count: usize,
}

fn bear_nav_context(bear: &den_runtime::bears::Bear, active: &str) -> minijinja::Value {
    context! {
        bear,
        bear_nav_active => active,
    }
}

async fn session_user(auth_session: &AuthSession) -> Result<&SessionUser, CustomError> {
    auth_session
        .user
        .as_ref()
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))
}

async fn load_session_bear(
    state: &AppState,
    auth_session: &AuthSession,
    slug: &str,
) -> Result<Result<(den_runtime::bears::Bear, bool), Redirect>, CustomError> {
    let user = session_user(auth_session).await?;
    if let Some(r) = email_verify_redirect(state.sqlx_pool(), user.id).await? {
        return Ok(Err(r));
    }
    let bear = load_bear_member(state.sqlx_pool(), user.id, slug).await?;
    let can_manage_bear = viewer_can_manage_bear(state.sqlx_pool(), user, bear.id).await?;
    Ok(Ok((bear, can_manage_bear)))
}

async fn load_session_bear_manage(
    state: &AppState,
    auth_session: &AuthSession,
    slug: &str,
) -> Result<Result<den_runtime::bears::Bear, Redirect>, CustomError> {
    let (bear, can_manage) = match load_session_bear(state, auth_session, slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(Err(r)),
    };
    if !can_manage {
        return Err(CustomError::Authorization(
            "bear admin role required".to_string(),
        ));
    }
    Ok(Ok(bear))
}

async fn memory_stats_for_bear(
    state: &AppState,
    bear_id: Uuid,
) -> Result<Option<BearMemoryAdminStats>, CustomError> {
    let manager = MemoryStoreManager::new(state.config.as_ref());
    match bear_memory_admin_stats(&manager, state.config.as_ref(), bear_id).await {
        Ok(stats) => Ok(Some(stats)),
        Err(err) => {
            tracing::warn!(%bear_id, "bear memory stats unavailable: {err}");
            Ok(None)
        }
    }
}

async fn overview_view(
    Path(slug): Path<String>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let id = bear.id;
    let member_count = bears_db::count_bear_members(state.sqlx_pool(), id).await?;
    let letta_configured = false;
    let agent_health_rows = bear_agent_health_rows(&state, id, letta_configured).await?;
    let roles_ready = agent_health_rows
        .iter()
        .filter(|row| row.health_status == "ok")
        .count();
    let roles_error = agent_health_rows
        .iter()
        .filter(|row| row.health_status == "error")
        .count();
    let memory_stats = {
        let manager = MemoryStoreManager::new(state.config.as_ref());
        match bear_memory_admin_stats(&manager, state.config.as_ref(), id).await {
            Ok(stats) => Some(stats),
            Err(err) => {
                tracing::warn!(%id, "hub memory stats unavailable: {err}");
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
        &state,
        "bear/settings/overview.html",
        auth_session,
        context! {
            message => query.message,
            member_count,
            native_runtime => true,
            context_profile_enabled => bear.context_profile.is_some(),
            letta_configured,
            agent_health_rows,
            roles_ready,
            roles_error,
            memory_stats,
            conversation_count,
            can_manage_bear,
            bear_nav_active => "overview",
            ..bear_nav_context(&bear, "overview"),
        },
    )
    .await
}

async fn access_view(
    Path(slug): Path<String>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let members: Vec<BearMemberAdminRow> = bears_db::list_members_for_bear(state.sqlx_pool(), bear.id)
        .await?
        .into_iter()
        .map(|m| BearMemberAdminRow {
            role_label: membership_role_label(m.role.as_deref()),
            user_id: m.user_id,
            username: m.username,
            display_name: m.display_name,
            role: m.role,
        })
        .collect();
    web::render_template(
        &state,
        "bear/settings/access.html",
        auth_session,
        context! {
            members,
            message => query.message,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "access"),
        },
    )
    .await
}

async fn persona_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let id = bear.id;
    let context_profile_enabled = bear.context_profile.is_some();
    let template_id = context_profile_from_json(&bear.context_profile)?
        .and_then(|p| p.template_id);
    let compiled: Option<BearCompiledConfigRow> =
        get_compiled_bear_config(state.sqlx_pool(), id).await?;
    let mut compiled_roles: Vec<CompiledRolePromptRow> = Vec::new();
    if let Some(ref row) = compiled {
        if let Ok(prompts) = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(
            row.rendered_prompts_json.0.clone(),
        ) {
            for role in ["chat", "pair", "curate", "work", "watch"] {
                if let Some(text) = prompts.get(role).and_then(|v| v.as_str()) {
                    let preview: String = text.chars().take(600).collect();
                    compiled_roles.push(CompiledRolePromptRow {
                        role: role.to_string(),
                        prompt_preview: preview,
                        char_count: text.len(),
                    });
                }
            }
        }
    }
    let block_bindings = list_bear_block_bindings(state.sqlx_pool(), id).await?;
    web::render_template(
        &state,
        "bear/settings/persona.html",
        auth_session,
        context! {
            context_profile_enabled,
            template_id,
            compiled,
            compiled_roles,
            block_bindings,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "persona"),
        },
    )
    .await
}

async fn profiles_view(
    Path(slug): Path<String>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let letta_configured = false;
    let agent_health_rows: Vec<BearProfileBindingHealthRow> =
        bear_agent_health_rows(&state, bear.id, letta_configured).await?;
    web::render_template(
        &state,
        "bear/settings/profiles.html",
        auth_session,
        context! {
            agent_health_rows,
            message => query.message,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "profiles"),
        },
    )
    .await
}

async fn profile_detail_view(
    Path((slug, profile)): Path<(String, String)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let role = profile
        .parse::<BearProfile>()
        .map_err(|err| CustomError::NotFound(err.to_string()))?;
    let role_detail = build_role_detail_view(&state, &bear, role).await?;
    web::render_template(
        &state,
        "bear/settings/profile.html",
        auth_session,
        context! {
            role_detail,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "profiles"),
        },
    )
    .await
}

async fn memory_view(
    Path(slug): Path<String>,
    Query(query): Query<MemoryQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let id = bear.id;
    let stats = memory_stats_for_bear(&state, id).await?;
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let store = manager.store_for_bear(id).await?;
    let requested_role = query.role.as_deref().unwrap_or("pair");
    let selected_role = requested_role
        .parse::<BearProfile>()
        .unwrap_or(BearProfile::Pair);
    let selected_role_name = selected_role.as_str().to_string();
    let search_query = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let selected_path = query.path.as_deref().map(str::trim).filter(|s| !s.is_empty());

    bears_db::ensure_bear_profile_binding_rows(state.sqlx_pool(), id).await?;
    let agents = bears_db::list_bear_profile_bindings(state.sqlx_pool(), id).await?;
    let mut role_rows = Vec::new();
    for agent in agents {
        let role = agent
            .parsed_profile()
            .map_err(|err| CustomError::System(format!("invalid bear agent role in DB: {err}")))?;
        let mut row = MemoryRoleRow {
            profile: role.as_str().to_string(),
            label: role_memory_label(role).to_string(),
            description: match role {
                BearProfile::Chat => "Notes and local memory from chat-like conversations.",
                BearProfile::Pair => "Coding collaboration notes, logs, decisions, and summaries.",
                BearProfile::Curate => "Review, reflection, and memory integration work.",
                BearProfile::Work => "Task execution logs, decisions, and summaries.",
                BearProfile::Watch => "Event/subscription logs and summaries.",
            }
            .to_string(),
            runtime_family: role.runtime_family().to_string(),
            selected: role == selected_role,
            status_label: "Unavailable".to_string(),
            file_count: 0,
            error: None,
        };
        match sqlite_memory::sqlite_memory_status(&store, role.as_str()).await {
            Ok(status) => {
                let available = status
                    .get("ok")
                    .and_then(|v| v.as_bool())
                    .or_else(|| status.get("available").and_then(|v| v.as_bool()))
                    .unwrap_or(true);
                row.status_label = if available {
                    "Available"
                } else {
                    "Unavailable"
                }
                .to_string();
                row.file_count = status
                    .get("file_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
            }
            Err(err) => {
                row.status_label = "Error".to_string();
                row.error = Some(err.to_string());
            }
        }
        role_rows.push(row);
    }

    let selected_tree =
        match sqlite_memory::sqlite_memory_browse(&store, selected_role.as_str()).await {
            Ok(v) => {
                let files = v
                    .get("children")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]));
                Some(serde_json::json!({ "files": files }))
            }
            Err(err) => {
                tracing::warn!(bear_id = %id, role = selected_role.as_str(), "Could not load memory tree: {err}");
                None
            }
        };

    let search_results = if let Some(q) = search_query {
        match sqlite_memory::sqlite_memory_search(&store, selected_role.as_str(), q, 50).await {
            Ok(v) => {
                let hits = v
                    .get("hits")
                    .and_then(|h| h.as_array())
                    .cloned()
                    .unwrap_or_default();
                let results: Vec<serde_json::Value> = hits
                    .iter()
                    .map(|h| {
                        let snippet = h.get("snippet").and_then(|s| s.as_str()).unwrap_or("");
                        serde_json::json!({
                            "path": h.get("path").and_then(|p| p.as_str()).unwrap_or(""),
                            "title": serde_json::Value::Null,
                            "snippet": snippet,
                            "size_bytes": snippet.len(),
                        })
                    })
                    .collect();
                let result_count = results.len();
                Some(serde_json::json!({
                    "results": results,
                    "result_count": result_count,
                    "scanned_file_count": result_count,
                }))
            }
            Err(err) => {
                tracing::warn!(bear_id = %id, role = selected_role.as_str(), "Could not search memory: {err}");
                None
            }
        }
    } else {
        None
    };

    let selected_file = if let Some(path) = selected_path {
        match sqlite_memory::sqlite_memory_read(&store, path).await {
            Ok(v) if v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) => {
                Some(serde_json::json!({
                    "path": v.get("path").and_then(|p| p.as_str()).unwrap_or(path),
                    "content": v.get("content").and_then(|c| c.as_str()).unwrap_or(""),
                }))
            }
            Ok(_) | Err(_) => None,
        }
    } else {
        None
    };

    let paths = list_all_logical_paths(&manager, id).await.unwrap_or_default();
    let recent = list_recent_memory_records(&manager, id, 12)
        .await
        .unwrap_or_default();
    let proposals = list_memory_proposals(&store, None, 20)
        .await
        .unwrap_or_default();
    let pair_reflection_runs = pair_reflection::list_recent_for_bear(state.sqlx_pool(), id, 10)
        .await
        .unwrap_or_default();
    let memory_proposals =
        memory_proposals::list_for_bear(state.sqlx_pool(), id, None, 25)
            .await
            .unwrap_or_default();

    web::render_template(
        &state,
        "bear/settings/memory.html",
        auth_session,
        context! {
            stats,
            role_rows,
            selected_role => selected_role_name,
            search_query => search_query.unwrap_or(""),
            selected_path => selected_path.unwrap_or(""),
            selected_tree,
            search_results,
            selected_file,
            paths,
            recent,
            proposals,
            pair_reflection_runs,
            memory_proposals,
            delete_notice => query.deleted,
            review_notice => query.review_requested,
            delete_error => query.error.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

async fn memory_delete_post(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<MemoryDeleteForm>,
) -> Result<Response, CustomError> {
    let user = session_user(&auth_session).await?;
    if let Some(r) = email_verify_redirect(state.sqlx_pool(), user.id).await? {
        return Ok(r.into_response());
    }
    let bear = load_bear_member(state.sqlx_pool(), user.id, &slug).await?;
    if !viewer_can_manage_bear(state.sqlx_pool(), user, bear.id).await? {
        return Err(CustomError::Authorization(
            "bear admin role required".to_string(),
        ));
    }
    let role = form
        .role
        .parse::<BearProfile>()
        .map_err(CustomError::ValidationError)?;
    let action = form.action.as_deref().unwrap_or("delete").trim();
    let confirm = form.confirm.trim();
    let mut paths = form
        .paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let memory_base = format!("/bear/{}/memory", bear.slug);
    if paths.is_empty() {
        return Ok(Redirect::to(&format!(
            "{memory_base}?role={}&error={}",
            role.as_str(),
            urlencoding::encode("Select at least one memory file.")
        ))
        .into_response());
    }
    if action == "request_review" {
        let title = form
            .review_title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Review selected memory");
        let summary = form
            .review_summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Selected memory files were marked for Reflection/curate review from the Bear memory UI.");
        let proposal = memory_proposals::create(
            state.sqlx_pool(),
            CreateMemoryProposal {
                bear_id: bear.id,
                source_profile: role,
                source_agent_id: bears_db::profile_binding_id(state.sqlx_pool(), bear.id, role)
                    .await?,
                source_paths: paths,
                source_refs: serde_json::json!([]),
                suggested_action: form
                    .suggested_action
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unspecified"),
                target_ref: None,
                title,
                summary,
                rationale: form.review_rationale.as_deref().map(str::trim).unwrap_or(""),
                proposed_content: None,
                proposed_patch: None,
                refs: serde_json::json!({}),
                sensitivity: form
                    .sensitivity
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("normal"),
                requires_human: form.requires_human.as_deref() == Some("on"),
                project_to_conversation: true,
            },
        )
        .await?;
        return Ok(Redirect::to(&format!(
            "{memory_base}?role={}&review_requested=1&path={}",
            role.as_str(),
            urlencoding::encode(
                proposal
                    .source_paths
                    .first()
                    .map(String::as_str)
                    .unwrap_or("")
            )
        ))
        .into_response());
    }
    if confirm != role.as_str() && confirm != bear.slug {
        return Ok(Redirect::to(&format!(
            "{memory_base}?role={}&error={}",
            role.as_str(),
            urlencoding::encode("Type the profile name or Bear slug to confirm deletion.")
        ))
        .into_response());
    }
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let store = manager.store_for_bear(bear.id).await?;
    let mut deleted = 0usize;
    for path in &paths {
        let result = sqlx::query(
            "DELETE FROM memory_records WHERE bear_id = ? AND scope_profile = ? AND logical_path = ?",
        )
        .bind(bear.id.to_string())
        .bind(role.as_str())
        .bind(path)
        .execute(store.pool())
        .await
        .map_err(|err| CustomError::System(format!("delete memory records failed: {err}")))?;
        if result.rows_affected() > 0 {
            deleted += 1;
        }
    }
    Ok(Redirect::to(&format!(
        "{memory_base}?role={}&deleted={deleted}",
        role.as_str()
    ))
    .into_response())
}

async fn memory_proposal_get(
    Path((slug, proposal_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let user = session_user(&auth_session).await?;
    if let Some(r) = email_verify_redirect(state.sqlx_pool(), user.id).await? {
        return Ok(r.into_response());
    }
    let bear = load_bear_member(state.sqlx_pool(), user.id, &slug).await?;
    let can_manage_bear = viewer_can_manage_bear(state.sqlx_pool(), user, bear.id).await?;
    let proposal = memory_proposals::get_for_bear(state.sqlx_pool(), bear.id, proposal_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("memory proposal not found".to_string()))?;
    web::render_template(
        &state,
        "bear/memory_proposal.html",
        auth_session,
        context! {
            bear,
            proposal,
            can_manage_bear,
            errors => None::<String>,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

async fn memory_proposal_post(
    Path((slug, proposal_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<MemoryProposalResolutionForm>,
) -> Result<Response, CustomError> {
    let user = session_user(&auth_session).await?;
    if let Some(r) = email_verify_redirect(state.sqlx_pool(), user.id).await? {
        return Ok(r.into_response());
    }
    let bear = load_bear_member(state.sqlx_pool(), user.id, &slug).await?;
    if !viewer_can_manage_bear(state.sqlx_pool(), user, bear.id).await? {
        return Err(CustomError::Authorization(
            "bear admin role required".to_string(),
        ));
    }
    let status = form.status.trim();
    if !matches!(
        status,
        "rejected" | "retained_local" | "deferred" | "superseded" | "needs_human_review"
    ) {
        return Err(CustomError::ValidationError(
            "invalid memory proposal status".to_string(),
        ));
    }
    memory_proposals::resolve_for_bear(
        state.sqlx_pool(),
        memory_proposals::ProposalResolutionParams {
            bear_id: bear.id,
            proposal_id,
            reviewer_profile: BearProfile::Curate,
            reviewer_agent_id: None,
            status,
            review_notes: form.review_notes.as_deref(),
            decision_summary: form.decision_summary.as_deref(),
            result_path: None,
            result_commit: None,
            project_to_conversation: true,
        },
    )
    .await?;
    Ok(Redirect::to(&format!(
        "/bear/{}/memory/proposals/{proposal_id}",
        bear.slug
    ))
    .into_response())
}

async fn memory_record_view(
    Path((slug, memory_id)): Path<(String, String)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let record = get_memory_record_by_id(&manager, bear.id, &memory_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("memory record not found".to_string()))?;
    web::render_template(
        &state,
        "bear/settings/memory_record.html",
        auth_session,
        context! {
            record,
            memory_id,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

async fn conversations_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let rows = conversation_persistence::list_conversations_for_bear(state.sqlx_pool(), bear.id, 50)
        .await?;
    let conversations: Vec<ConversationAdminRow> = rows
        .into_iter()
        .map(|c| ConversationAdminRow {
            id: c.id,
            external_id: c
                .external_conversation_id
                .unwrap_or_else(|| "(none)".to_string()),
            title: c
                .current_title
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "Untitled".to_string()),
            source_session: c
                .source_acp_session_id
                .unwrap_or_else(|| "—".to_string()),
            updated_at: c.updated_at.to_string(),
        })
        .collect();
    web::render_template(
        &state,
        "bear/settings/conversations.html",
        auth_session,
        context! {
            conversations,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "conversations"),
        },
    )
    .await
}

async fn conversation_detail_view(
    Path((slug, conversation_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let conv = conversation_persistence::get_conversation_by_id(state.sqlx_pool(), conversation_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("conversation not found".to_string()))?;
    if conv.bear_id != bear.id {
        return Err(CustomError::NotFound("conversation not found".to_string()));
    }
    let messages = list_messages_page(state.sqlx_pool(), conversation_id, None, 40).await?;
    let message_rows: Vec<MessageAdminRow> = messages
        .into_iter()
        .rev()
        .map(|m| {
            let preview: String = m.content_text.chars().take(280).collect();
            MessageAdminRow {
                sequence_no: m.sequence_no,
                message_type: m.message_type,
                role: m.role.unwrap_or_else(|| "—".to_string()),
                visibility: m.visibility,
                preview,
            }
        })
        .collect();
    web::render_template(
        &state,
        "bear/settings/conversation.html",
        auth_session,
        context! {
            conv,
            message_rows,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "conversations"),
        },
    )
    .await
}

async fn context_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let mut prompt_blocks: Vec<PromptMemoryAdminRow> = Vec::new();
    for role in ["pair", "chat", "curate", "work", "watch"] {
        if let Ok(blocks) =
            list_prompt_memory_blocks_for_bear_profile(state.sqlx_pool(), bear.id, role).await
        {
            for block in blocks {
                let body_preview: String = block.body.chars().take(200).collect();
                prompt_blocks.push(PromptMemoryAdminRow {
                    block_id: block.id,
                    scope: format!("{:?}", block.scope),
                    block_type: format!("{:?}", block.block_type),
                    state: format!("{:?}", block.state),
                    title: block.title,
                    body_preview,
                });
            }
        }
    }
    web::render_template(
        &state,
        "bear/settings/context.html",
        auth_session,
        context! {
            prompt_blocks,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "context"),
        },
    )
    .await
}

async fn policy_view(
    Path(slug): Path<String>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let id = bear.id;
    let web_sources: Vec<BearWebSourceRow> = bear_web_sources(state.sqlx_pool(), id).await?;
    let web_approvals: Vec<BearWebApprovalRow> = bear_web_approvals(state.sqlx_pool(), id).await?;
    let web_fetches: Vec<BearWebFetchRow> = bear_web_fetches(state.sqlx_pool(), id).await?;
    let plan_mode_rows: Vec<BearPlanModeRow> = bear_plan_mode_rows(state.sqlx_pool(), id).await?;
    web::render_template(
        &state,
        "bear/settings/policy.html",
        auth_session,
        context! {
            web_sources,
            web_approvals,
            web_fetches,
            plan_mode_rows,
            message => query.message,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "policy"),
        },
    )
    .await
}

async fn advanced_view(
    Path(slug): Path<String>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let stats = memory_stats_for_bear(&state, bear.id).await?;
    web::render_template(
        &state,
        "bear/settings/advanced.html",
        auth_session,
        context! {
            stats,
            letta_configured => false,
            message => query.message,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "advanced"),
        },
    )
    .await
}

async fn grant_member_action(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<MemberGrantForm>,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(b) => b,
        Err(r) => return Ok(r.into_response()),
    };
    let target_id = if let Some(user_id) = form.user_id.filter(|id| *id > 0) {
        user_id
    } else {
        let uname = form.username.trim();
        if uname.is_empty() {
            return Ok(Redirect::to(&format!(
                "/bear/{}/access?message={}",
                bear.slug,
                urlencoding::encode("Username is required.")
            ))
            .into_response());
        }
        match user_db::get_user_by_username(state.sqlx_pool(), uname).await? {
            Some(u) => u.id,
            None => {
                return Ok(Redirect::to(&format!(
                    "/bear/{}/access?message={}",
                    bear.slug,
                    urlencoding::encode("User not found.")
                ))
                .into_response());
            }
        }
    };
    let role = form.role.trim();
    let role_opt = match role {
        "" | "member" => Some(BEAR_ROLE_MEMBER),
        "admin" => Some(BEAR_ROLE_ADMIN),
        other => Some(other),
    };
    bears_db::grant_membership(state.sqlx_pool(), target_id, bear.id, role_opt).await?;
    Ok(Redirect::to(&format!(
        "/bear/{}/access?message={}",
        bear.slug,
        urlencoding::encode("Access granted.")
    ))
    .into_response())
}

async fn revoke_member_action(
    Path((slug, user_id)): Path<(String, i32)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(b) => b,
        Err(r) => return Ok(r.into_response()),
    };
    if role_is_bear_admin(
        bears_db::membership_role_for_user(state.sqlx_pool(), user_id, bear.id)
            .await?
            .flatten()
            .as_deref(),
    ) {
        let n = bears_db::count_bear_admins(state.sqlx_pool(), bear.id).await?;
        if n <= 1 {
            return Ok(Redirect::to(&format!(
                "/bear/{}/access?message={}",
                bear.slug,
                urlencoding::encode("Cannot remove the last bear admin.")
            ))
            .into_response());
        }
    }
    match bears_db::revoke_membership(state.sqlx_pool(), user_id, bear.id).await {
        Ok(()) => Ok(Redirect::to(&format!(
            "/bear/{}/access?message={}",
            bear.slug,
            urlencoding::encode("Access removed.")
        ))
        .into_response()),
        Err(DenError::NotFound(_)) => Ok(Redirect::to(&format!(
            "/bear/{}/access?message={}",
            bear.slug,
            urlencoding::encode("Membership not found.")
        ))
        .into_response()),
        Err(err) => Err(err.into()),
    }
}

async fn add_web_source_action(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<AddWebSourceForm>,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(b) => b,
        Err(r) => return Ok(r.into_response()),
    };
    let id = bear.id;
    let scope_kind = form.scope_kind.trim();
    let policy = form.policy.trim();
    if !matches!(scope_kind, "host" | "url")
        || !matches!(policy, "preferred" | "allowed" | "blocked")
    {
        return Ok(Redirect::to(&format!(
            "/bear/{}/policy?message={}",
            bear.slug,
            urlencoding::encode("Invalid web source policy form.")
        ))
        .into_response());
    }
    let scope_value = match web_policy::normalize_web_scope_value(scope_kind, &form.scope_value) {
        Ok(scope_value) => scope_value,
        Err(err) => {
            return Ok(Redirect::to(&format!(
                "/bear/{}/policy?message={}",
                bear.slug,
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
        "/bear/{}/policy?message={}",
        bear.slug,
        urlencoding::encode("Web source saved.")
    ))
    .into_response())
}

async fn delete_web_source_action(
    Path((slug, source_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(b) => b,
        Err(r) => return Ok(r.into_response()),
    };
    sqlx::query("DELETE FROM bear_web_sources WHERE bear_id = $1 AND id = $2")
        .bind(bear.id)
        .bind(source_id)
        .execute(state.sqlx_pool())
        .await?;
    Ok(Redirect::to(&format!(
        "/bear/{}/policy?message={}",
        bear.slug,
        urlencoding::encode("Web source deleted.")
    ))
    .into_response())
}

async fn add_web_approval_action(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<AddWebApprovalForm>,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(b) => b,
        Err(r) => return Ok(r.into_response()),
    };
    let scope_kind = form.scope_kind.trim();
    if !matches!(scope_kind, "host" | "url") {
        return Ok(Redirect::to(&format!(
            "/bear/{}/policy?message={}",
            bear.slug,
            urlencoding::encode("Invalid web approval scope.")
        ))
        .into_response());
    }
    let scope_value = match web_policy::normalize_web_scope_value(scope_kind, &form.scope_value) {
        Ok(scope_value) => scope_value,
        Err(err) => {
            return Ok(Redirect::to(&format!(
                "/bear/{}/policy?message={}",
                bear.slug,
                urlencoding::encode(&err.to_string())
            ))
            .into_response());
        }
    };
    web_policy::record_web_approval(
        state.sqlx_pool(),
        bear.id,
        scope_kind,
        &scope_value,
        auth_session.user.as_ref().map(|u| u.id),
        "bear_admin",
        None,
    )
    .await?;
    Ok(Redirect::to(&format!(
        "/bear/{}/policy?message={}",
        bear.slug,
        urlencoding::encode("Web approval added.")
    ))
    .into_response())
}

async fn revoke_web_approval_action(
    Path((slug, approval_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(b) => b,
        Err(r) => return Ok(r.into_response()),
    };
    sqlx::query("UPDATE bear_web_approvals SET revoked_at = now() WHERE bear_id = $1 AND id = $2")
        .bind(bear.id)
        .bind(approval_id)
        .execute(state.sqlx_pool())
        .await?;
    Ok(Redirect::to(&format!(
        "/bear/{}/policy?message={}",
        bear.slug,
        urlencoding::encode("Web approval revoked.")
    ))
    .into_response())
}

async fn provision_missing_roles_action(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(b) => b,
        Err(r) => return Ok(r.into_response()),
    };
    let message = match provision::provision_missing_bear_profiles(
        state.sqlx_pool(),
        state.config.as_ref(),
        bear.id,
    )
    .await
    {
        Ok(0) => "No missing native profile bindings to provision.".to_string(),
        Ok(n) => format!("Provisioned {n} missing native profile binding(s)."),
        Err(err) => format!("Provisioning failed: {err}"),
    };
    Ok(Redirect::to(&format!(
        "/bear/{}/profiles?message={}",
        bear.slug,
        urlencoding::encode(&message)
    ))
    .into_response())
}
