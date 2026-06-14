//! Progressive-disclosure admin views for a single bear.

use axum::{
    extract::{Path, Query, State},
    response::Response,
    routing::get,
    Router,
};
use axum_extra::routing::RouterExt;
use minijinja::context;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth_backend::AuthSession,
    errors::CustomError,
    web::{self, AppState},
    core::user::db as user_db,
};
use den_runtime::{
    bears::{
            context_profile_from_json, db as bears_db,             get_compiled_bear_config, list_bear_block_bindings,
            managed_blocks::BearCompiledConfigRow,
        },
    conversation_persistence::{self, list_messages_page},
    memory::{
            admin_inspect::{
                bear_memory_admin_stats, get_memory_record_by_id, list_all_logical_paths,
                list_recent_memory_records,
            },
            tools as sqlite_memory, MemoryStoreManager,
        },
    prompt_memory_block_store::list_prompt_memory_blocks_for_bear_profile,
};

use super::bears::{
    bear_agent_health_rows, bear_plan_mode_rows, bear_web_approvals, bear_web_fetches,
    bear_web_sources, membership_role_label, BearMemberAdminRow, BearProfileBindingHealthRow,
    BearPlanModeRow, BearWebApprovalRow, BearWebFetchRow, BearWebSourceRow,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route_with_tsr("/bears/{id}/access", get(access_view))
        .route_with_tsr("/bears/{id}/persona", get(persona_view))
        .route_with_tsr("/bears/{id}/roles", get(roles_view))
        .route_with_tsr("/bears/{id}/memory", get(memory_view))
        .route_with_tsr(
            "/bears/{id}/memory/records/{memory_id}",
            get(memory_record_view),
        )
        .route_with_tsr("/bears/{id}/conversations", get(conversations_view))
        .route_with_tsr(
            "/bears/{id}/conversations/{conversation_id}",
            get(conversation_detail_view),
        )
        .route_with_tsr("/bears/{id}/context", get(context_view))
        .route_with_tsr("/bears/{id}/policy", get(policy_view))
        .route_with_tsr("/bears/{id}/advanced", get(advanced_view))
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
    path: Option<String>,
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

async fn load_bear(state: &AppState, id: Uuid) -> Result<den_runtime::bears::Bear, CustomError> {
    bears_db::get_bear(state.sqlx_pool(), id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))
}

fn bear_nav_context(bear: &den_runtime::bears::Bear, active: &str) -> minijinja::Value {
    context! {
        bear,
        bear_nav_active => active,
    }
}

async fn memory_stats_for_bear(
    state: &AppState,
    bear_id: Uuid,
) -> Result<Option<den_runtime::memory::BearMemoryAdminStats>, CustomError> {
    let manager = MemoryStoreManager::new(state.config.as_ref());
    match bear_memory_admin_stats(&manager, state.config.as_ref(), bear_id).await {
        Ok(stats) => Ok(Some(stats)),
        Err(err) => {
            tracing::warn!(%bear_id, "admin memory stats unavailable: {err}");
            Ok(None)
        }
    }
}

async fn access_view(
    Path(id): Path<Uuid>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let members: Vec<BearMemberAdminRow> = bears_db::list_members_for_bear(state.sqlx_pool(), id)
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
    let users = user_db::get_users(state.sqlx_pool()).await?;
    web::render_template(
        &state,
        "admin/bears/access.html",
        auth_session,
        context! {
            members,
            users,
            message => query.message,
            native_runtime => true,
            ..bear_nav_context(&bear, "access"),
        },
    )
    .await
}

async fn persona_view(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let context_profile_enabled = bear.context_profile.is_some();
    let template_id = context_profile_from_json(&bear.context_profile)?
        .and_then(|p| p.template_id);
    let compiled: Option<BearCompiledConfigRow> = get_compiled_bear_config(state.sqlx_pool(), id).await?;
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
        "admin/bears/persona.html",
        auth_session,
        context! {
            context_profile_enabled,
            template_id,
            compiled,
            compiled_roles,
            block_bindings,
            native_runtime => true,
            ..bear_nav_context(&bear, "persona"),
        },
    )
    .await
}

async fn roles_view(
    Path(id): Path<Uuid>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let letta_configured = false;
    let agent_health_rows: Vec<BearProfileBindingHealthRow> =
        bear_agent_health_rows(&state, id, letta_configured).await?;
    web::render_template(
        &state,
        "admin/bears/roles.html",
        auth_session,
        context! {
            agent_health_rows,
            letta_configured,
            message => query.message,
            native_runtime => true,
            ..bear_nav_context(&bear, "roles"),
        },
    )
    .await
}

async fn memory_view(
    Path(id): Path<Uuid>,
    Query(query): Query<MemoryQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let stats = memory_stats_for_bear(&state, id).await?;
    let role = query
        .role
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("pair");
    let paths = list_all_logical_paths(&manager, id).await.unwrap_or_default();
    let recent = list_recent_memory_records(&manager, id, 12)
        .await
        .unwrap_or_default();
    let proposals = if let Ok(store) = manager.store_for_bear(id).await {
        den_runtime::memory::store::list_memory_proposals(&store, None, 20)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let path_content = if let Some(path) = query.path.as_deref().filter(|p| !p.is_empty()) {
        if let Ok(store) = manager.store_for_bear(id).await {
            sqlite_memory::sqlite_memory_read(&store, path).await.ok()
        } else {
            None
        }
    } else {
        None
    };
    web::render_template(
        &state,
        "admin/bears/memory.html",
        auth_session,
        context! {
            stats,
            role,
            paths,
            recent,
            proposals,
            path_content,
            selected_path => query.path,
            native_runtime => true,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

async fn memory_record_view(
    Path((id, memory_id)): Path<(Uuid, String)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let record = get_memory_record_by_id(&manager, id, &memory_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("memory record not found".to_string()))?;
    web::render_template(
        &state,
        "admin/bears/memory_record.html",
        auth_session,
        context! {
            record,
            memory_id,
            native_runtime => true,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

async fn conversations_view(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let rows = conversation_persistence::list_conversations_for_bear(state.sqlx_pool(), id, 50)
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
        "admin/bears/conversations.html",
        auth_session,
        context! {
            conversations,
            native_runtime => true,
            ..bear_nav_context(&bear, "conversations"),
        },
    )
    .await
}

async fn conversation_detail_view(
    Path((id, conversation_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let conv = conversation_persistence::get_conversation_by_id(state.sqlx_pool(), conversation_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("conversation not found".to_string()))?;
    if conv.bear_id != id {
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
        "admin/bears/conversation.html",
        auth_session,
        context! {
            conv,
            message_rows,
            native_runtime => true,
            ..bear_nav_context(&bear, "conversations"),
        },
    )
    .await
}

async fn context_view(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let mut prompt_blocks: Vec<PromptMemoryAdminRow> = Vec::new();
    for role in ["pair", "chat", "curate", "work", "watch"] {
        if let Ok(blocks) =
            list_prompt_memory_blocks_for_bear_profile(state.sqlx_pool(), id, role).await
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
        "admin/bears/context.html",
        auth_session,
        context! {
            prompt_blocks,
            native_runtime => true,
            ..bear_nav_context(&bear, "context"),
        },
    )
    .await
}

async fn policy_view(
    Path(id): Path<Uuid>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let web_sources: Vec<BearWebSourceRow> = bear_web_sources(state.sqlx_pool(), id).await?;
    let web_approvals: Vec<BearWebApprovalRow> = bear_web_approvals(state.sqlx_pool(), id).await?;
    let web_fetches: Vec<BearWebFetchRow> = bear_web_fetches(state.sqlx_pool(), id).await?;
    let plan_mode_rows: Vec<BearPlanModeRow> = bear_plan_mode_rows(state.sqlx_pool(), id).await?;
    web::render_template(
        &state,
        "admin/bears/policy.html",
        auth_session,
        context! {
            web_sources,
            web_approvals,
            web_fetches,
            plan_mode_rows,
            message => query.message,
            native_runtime => true,
            ..bear_nav_context(&bear, "policy"),
        },
    )
    .await
}

async fn advanced_view(
    Path(id): Path<Uuid>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = load_bear(&state, id).await?;
    let letta_configured = false;
    let stats = memory_stats_for_bear(&state, id).await?;
    web::render_template(
        &state,
        "admin/bears/advanced.html",
        auth_session,
        context! {
            stats,
            letta_configured,
            message => query.message,
            letta_retry_message => query.message,
            native_runtime => true,
            ..bear_nav_context(&bear, "advanced"),
        },
    )
    .await
}
