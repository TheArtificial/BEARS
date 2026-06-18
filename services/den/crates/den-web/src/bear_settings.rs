//! Bear-scoped settings at `/bear/{slug}/…` for members (read) and bear admins (write).

use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use axum_extra::extract::Form;
use axum_extra::routing::RouterExt;
use minijinja::context;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{Cursor, Read, Write};
use std::path::{Path as FsPath, PathBuf};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

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
        get_compiled_bear_config, list_bear_block_bindings,
        managed_blocks::BearCompiledConfigRow,
        provision, BearProfile,
    },
    conversation_persistence::{self, list_messages_page},
    memory::{admin_inspect::bear_memory_admin_stats, BearMemoryAdminStats, MemoryStoreManager},
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
    bear_profile::build_role_detail_view,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route_with_tsr("/bear/{slug}/overview", get(overview_view))
        .route_with_tsr("/bear/{slug}/access", get(access_view))
        .route_with_tsr("/bear/{slug}/persona", get(persona_view))
        .route_with_tsr("/bear/{slug}/profiles", get(profiles_view))
        .route_with_tsr("/bear/{slug}/profiles/{profile}", get(profile_detail_view))
        .route_with_tsr("/bear/{slug}/conversations", get(conversations_view))
        .route_with_tsr(
            "/bear/{slug}/conversations/{conversation_id}",
            get(conversation_detail_view),
        )
        .route_with_tsr("/bear/{slug}/context", get(context_view))
        .route_with_tsr("/bear/{slug}/policy", get(policy_view))
        .route_with_tsr("/bear/{slug}/advanced", get(advanced_view))
        .route_with_tsr("/bear/{slug}/export.bear", get(export_bear_bundle))
        .route_with_tsr("/bears/import", post(import_bear_bundle))
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

const BEAR_BUNDLE_FORMAT: &str = "bear";
const BEAR_BUNDLE_VERSION: u32 = 1;
const BEAR_BUNDLE_MAX_UPLOAD_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct BearBundleManifest {
    format: String,
    version: u32,
    bear: BearBundleIdentity,
    prompts: BearBundlePrompts,
    #[serde(default)]
    profiles: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct BearBundleIdentity {
    slug: String,
    name: String,
    description: String,
    birthdate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tools_enabled: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    letta_agent_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BearBundlePrompts {
    system_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_profile: Option<serde_json::Value>,
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

pub(crate) fn bear_nav_context(bear: &den_runtime::bears::Bear, active: &str) -> minijinja::Value {
    context! {
        bear,
        bear_nav_active => active,
    }
}

pub(crate) async fn session_user(auth_session: &AuthSession) -> Result<&SessionUser, CustomError> {
    auth_session
        .user
        .as_ref()
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))
}

pub(crate) async fn load_session_bear(
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

pub(crate) async fn load_session_bear_manage(
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

fn memory_sqlite_path(config: &den_core::config::Config, bear_id: Uuid) -> PathBuf {
    FsPath::new(&config.bear_sqlite_data_dir).join(format!("{bear_id}.sqlite"))
}

fn sqlite_string_literal(path: &FsPath) -> Result<String, CustomError> {
    let raw = path
        .to_str()
        .ok_or_else(|| CustomError::System("sqlite path is not valid UTF-8".to_string()))?;
    Ok(format!("'{}'", raw.replace('\'', "''")))
}

fn slug_base(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.trim().to_ascii_lowercase().chars() {
        let mapped = if ch.is_ascii_alphanumeric() { ch } else { '-' };
        if mapped == '-' {
            if !last_dash && !out.is_empty() {
                out.push('-');
                last_dash = true;
            }
        } else {
            out.push(mapped);
            last_dash = false;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "imported-bear".to_string()
    } else {
        trimmed
    }
}

async fn unique_import_slug(pool: &sqlx::PgPool, requested: &str) -> Result<String, CustomError> {
    let base = slug_base(requested);
    if !bears_db::bear_slug_exists(pool, &base).await? {
        return Ok(base);
    }
    for idx in 2..=999 {
        let candidate = format!("{base}-{idx}");
        if !bears_db::bear_slug_exists(pool, &candidate).await? {
            return Ok(candidate);
        }
    }
    Err(CustomError::ValidationError(
        "could not find available slug for imported Bear".to_string(),
    ))
}

fn manifest_for_bear(bear: &den_runtime::bears::Bear) -> Result<BearBundleManifest, CustomError> {
    let exported_birthdate = bear
        .created_at
        .format(&Rfc3339)
        .map_err(|err| CustomError::System(format!("format Bear birthdate failed: {err}")))?;
    Ok(BearBundleManifest {
        format: BEAR_BUNDLE_FORMAT.to_string(),
        version: BEAR_BUNDLE_VERSION,
        bear: BearBundleIdentity {
            slug: bear.slug.clone(),
            name: bear.name.clone(),
            description: bear.description.clone(),
            birthdate: exported_birthdate.chars().take(10).collect(),
            default_model: bear.default_model.clone(),
            tools_enabled: bear.tools_enabled.as_ref().map(|v| v.0.clone()),
            letta_agent_type: bear.letta_agent_type.clone(),
        },
        prompts: BearBundlePrompts {
            system_prompt: bear.system_prompt.clone(),
            context_profile: bear.context_profile.as_ref().map(|v| v.0.clone()),
        },
        profiles: json!({}),
    })
}

async fn snapshot_memory_sqlite(state: &AppState, bear_id: Uuid) -> Result<Vec<u8>, CustomError> {
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let store = manager.store_for_bear(bear_id).await?;
    let snapshot_path =
        std::env::temp_dir().join(format!("bear-export-{bear_id}-{}.sqlite", Uuid::new_v4()));
    if snapshot_path.exists() {
        let _ = std::fs::remove_file(&snapshot_path);
    }
    let literal = sqlite_string_literal(&snapshot_path)?;
    sqlx::query(&format!("VACUUM INTO {literal}"))
        .execute(store.pool())
        .await
        .map_err(|err| CustomError::System(format!("snapshot memory sqlite failed: {err}")))?;
    let bytes = std::fs::read(&snapshot_path)
        .map_err(|err| CustomError::System(format!("read memory sqlite snapshot failed: {err}")))?;
    if let Err(err) = std::fs::remove_file(&snapshot_path) {
        tracing::warn!(path = %snapshot_path.display(), error = %err, "failed to remove Bear export SQLite snapshot");
    }
    Ok(bytes)
}

fn build_bear_bundle(manifest_yaml: &str, memory_sqlite: &[u8]) -> Result<Vec<u8>, CustomError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer
        .start_file("bear.yaml", options)
        .map_err(|err| CustomError::System(format!("start bear.yaml in bundle failed: {err}")))?;
    writer
        .write_all(manifest_yaml.as_bytes())
        .map_err(|err| CustomError::System(format!("write bear.yaml to bundle failed: {err}")))?;
    writer.start_file("memory.sqlite", options).map_err(|err| {
        CustomError::System(format!("start memory.sqlite in bundle failed: {err}"))
    })?;
    writer.write_all(memory_sqlite).map_err(|err| {
        CustomError::System(format!("write memory.sqlite to bundle failed: {err}"))
    })?;
    let cursor = writer
        .finish()
        .map_err(|err| CustomError::System(format!("finish Bear bundle failed: {err}")))?;
    Ok(cursor.into_inner())
}

fn read_bear_bundle(bytes: &[u8]) -> Result<(BearBundleManifest, Vec<u8>), CustomError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| CustomError::ValidationError(format!("invalid .bear zip: {err}")))?;
    let mut manifest_yaml = String::new();
    archive
        .by_name("bear.yaml")
        .map_err(|_| CustomError::ValidationError(".bear bundle missing bear.yaml".to_string()))?
        .read_to_string(&mut manifest_yaml)
        .map_err(|err| CustomError::ValidationError(format!("read bear.yaml failed: {err}")))?;
    let mut memory_sqlite = Vec::new();
    archive
        .by_name("memory.sqlite")
        .map_err(|_| {
            CustomError::ValidationError(".bear bundle missing memory.sqlite".to_string())
        })?
        .read_to_end(&mut memory_sqlite)
        .map_err(|err| CustomError::ValidationError(format!("read memory.sqlite failed: {err}")))?;
    let manifest: BearBundleManifest = serde_yml::from_str(&manifest_yaml)
        .map_err(|err| CustomError::ValidationError(format!("parse bear.yaml failed: {err}")))?;
    if manifest.format != BEAR_BUNDLE_FORMAT || manifest.version != BEAR_BUNDLE_VERSION {
        return Err(CustomError::ValidationError(format!(
            "unsupported .bear format {} version {}",
            manifest.format, manifest.version
        )));
    }
    if memory_sqlite.is_empty() {
        return Err(CustomError::ValidationError(
            "memory.sqlite is empty".to_string(),
        ));
    }
    Ok((manifest, memory_sqlite))
}

async fn rewrite_imported_memory_bear_id(
    state: &AppState,
    bear_id: Uuid,
) -> Result<(), CustomError> {
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let store = manager.store_for_bear(bear_id).await?;
    let new_id = bear_id.to_string();
    for table in [
        "memory_records",
        "entities",
        "entity_handles",
        "memory_relations",
        "memory_access_rules",
        "memory_promotions",
        "memory_proposals",
        "memory_observations",
        "reflection_run_outcomes",
    ] {
        sqlx::query(&format!("UPDATE {table} SET bear_id = ?"))
            .bind(&new_id)
            .execute(store.pool())
            .await
            .map_err(|err| CustomError::System(format!("rewrite {table}.bear_id failed: {err}")))?;
    }
    let integrity: Vec<(String,)> = sqlx::query_as("PRAGMA integrity_check")
        .fetch_all(store.pool())
        .await
        .map_err(|err| {
            CustomError::System(format!("imported memory integrity check failed: {err}"))
        })?;
    if integrity.first().map(|row| row.0.as_str()) != Some("ok") {
        return Err(CustomError::ValidationError(format!(
            "imported memory.sqlite failed integrity check: {:?}",
            integrity
        )));
    }
    Ok(())
}

async fn export_bear_bundle(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(b) => b,
        Err(r) => return Ok(r.into_response()),
    };
    let manifest = manifest_for_bear(&bear)?;
    let manifest_yaml = serde_yml::to_string(&manifest)
        .map_err(|err| CustomError::System(format!("serialize bear.yaml failed: {err}")))?;
    let memory_sqlite = snapshot_memory_sqlite(&state, bear.id).await?;
    let bundle = build_bear_bundle(&manifest_yaml, &memory_sqlite)?;
    let filename = format!("{}.bear", slug_base(&bear.slug));

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(Body::from(bundle))
        .map_err(|err| CustomError::System(format!("build Bear export response failed: {err}")))
}

async fn import_bear_bundle(
    State(state): State<AppState>,
    auth_session: AuthSession,
    mut multipart: Multipart,
) -> Result<Response, CustomError> {
    let user = session_user(&auth_session).await?;
    if let Some(r) = email_verify_redirect(state.sqlx_pool(), user.id).await? {
        return Ok(r.into_response());
    }

    let mut bundle_bytes: Option<Vec<u8>> = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|err| CustomError::ValidationError(format!("invalid .bear upload: {err}")))?
    {
        if field.name() != Some("bundle") {
            continue;
        }
        let mut data = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(|err| {
            CustomError::ValidationError(format!("read .bear upload failed: {err}"))
        })? {
            if data.len() + chunk.len() > BEAR_BUNDLE_MAX_UPLOAD_BYTES {
                return Err(CustomError::ValidationError(
                    ".bear bundle exceeds the 256 MiB upload limit".to_string(),
                ));
            }
            data.extend_from_slice(&chunk);
        }
        bundle_bytes = Some(data);
        break;
    }

    let bundle_bytes = bundle_bytes
        .filter(|bytes| !bytes.is_empty())
        .ok_or_else(|| CustomError::ValidationError("please select a .bear bundle".to_string()))?;
    let (manifest, memory_sqlite) = read_bear_bundle(&bundle_bytes)?;
    let slug = unique_import_slug(state.sqlx_pool(), &manifest.bear.slug).await?;

    let bear_id = bears_db::create_bear_with_context_profile(
        state.sqlx_pool(),
        bears_db::BearParams {
            slug: &slug,
            name: &manifest.bear.name,
            description: &manifest.bear.description,
            system_prompt: &manifest.prompts.system_prompt,
            default_model: manifest.bear.default_model.as_deref(),
            tools_enabled: manifest.bear.tools_enabled.clone().map(sqlx::types::Json),
            letta_agent_type: manifest.bear.letta_agent_type.as_deref(),
            letta_tool_ids: sqlx::types::Json(Vec::new()),
            context_profile: manifest
                .prompts
                .context_profile
                .clone()
                .map(sqlx::types::Json),
        },
    )
    .await?;

    let birthdate = manifest.bear.birthdate.trim();
    if !birthdate.is_empty() {
        sqlx::query("UPDATE bears SET created_at = $1::date, updated_at = NOW() WHERE id = $2")
            .bind(birthdate)
            .bind(bear_id)
            .execute(state.sqlx_pool())
            .await
            .map_err(|err| {
                CustomError::ValidationError(format!("invalid Bear birthdate: {err}"))
            })?;
    }

    bears_db::grant_membership(state.sqlx_pool(), user.id, bear_id, Some(BEAR_ROLE_ADMIN)).await?;

    let memory_path = memory_sqlite_path(state.config.as_ref(), bear_id);
    if let Some(parent) = memory_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            CustomError::System(format!("create Bear memory directory failed: {err}"))
        })?;
    }
    std::fs::write(&memory_path, memory_sqlite).map_err(|err| {
        CustomError::System(format!("write imported memory.sqlite failed: {err}"))
    })?;
    rewrite_imported_memory_bear_id(&state, bear_id).await?;

    if let Err(err) =
        provision::provision_bear_if_configured(state.sqlx_pool(), state.config.as_ref(), bear_id)
            .await
    {
        tracing::warn!(%bear_id, error = %err, "provision after Bear import failed");
    }
    if let Err(err) =
        provision::reconcile_bear_native(state.sqlx_pool(), state.config.as_ref(), bear_id).await
    {
        tracing::warn!(%bear_id, error = %err, "reconcile after Bear import failed");
    }

    Ok(Redirect::to(&format!(
        "/bear/{slug}/overview?message={}",
        urlencoding::encode("Bear imported.")
    ))
    .into_response())
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
    let conversation_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM conversations WHERE bear_id = $1")
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
            letta_import_locked => memory_stats.as_ref().map(|stats| stats.record_count > 0).unwrap_or(true),
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
    let members: Vec<BearMemberAdminRow> =
        bears_db::list_members_for_bear(state.sqlx_pool(), bear.id)
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
    let template_id = context_profile_from_json(&bear.context_profile)?.and_then(|p| p.template_id);
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
        .map_err(CustomError::NotFound)?;
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

async fn conversations_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let rows =
        conversation_persistence::list_conversations_for_bear(state.sqlx_pool(), bear.id, 50)
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
            source_session: c.source_acp_session_id.unwrap_or_else(|| "—".to_string()),
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
        r"
        INSERT INTO bear_web_sources (bear_id, scope_kind, scope_value, label, policy, priority)
        VALUES ($1, $2, $3, NULLIF($4, ''), $5, $6)
        ON CONFLICT (bear_id, scope_kind, scope_value)
        DO UPDATE SET label = EXCLUDED.label,
                      policy = EXCLUDED.policy,
                      priority = EXCLUDED.priority,
                      updated_at = now()
        ",
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
