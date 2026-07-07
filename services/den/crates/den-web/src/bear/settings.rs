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
use axum_login::tower_sessions::Session;
use minijinja::context;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path as FsPath, PathBuf};
use std::str::FromStr;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

use crate::{
    auth_backend::{AuthSession, SessionUser},
    core::{user::db as user_db, web_policy},
    errors::CustomError,
    web::{self, AppState},
};
use den_core::{AgentLoopControlLevel, DenError};
use den_memory::{bear_memory_admin_stats, BearMemoryAdminStats, MemoryStoreManager};
use den_protocol::ContextBudgetReport;
use den_service::prompt_memory_block_store::list_prompt_memory_blocks_for_bear_profile;
use den_service::{
    bears::{
        context_profile_from_json, db as bears_db,
        db::{role_is_bear_admin, BEAR_ROLE_ADMIN, BEAR_ROLE_MEMBER},
        get_compiled_bear_config, list_bear_block_bindings,
        managed_blocks::BearCompiledConfigRow,
        provision, BearProfile,
    },
    conversation::persistence::{self as conversation_persistence, list_messages_page},
};

use crate::web::admin::bears::{
    bear_agent_health_rows, bear_plan_mode_rows, bear_web_approvals, bear_web_fetches,
    bear_web_sources, membership_role_label, AddWebApprovalForm, AddWebSourceForm,
    BearMemberAdminRow, BearPlanModeRow, BearProfileBindingHealthRow, BearWebApprovalRow,
    BearWebFetchRow, BearWebSourceRow,
};
use crate::web::bear::create_support::{
    all_model_catalog_options_context_for_bear, canonical_default_model_handle,
    provision_bifrost_virtual_key_for_bear,
};
use den_llm::ModelOption;

use super::{
    member::{email_verify_redirect, load_bear_member, viewer_can_manage_bear},
    profile::build_role_detail_view,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route_with_tsr("/bear/{slug}/overview", get(overview_view))
        .route_with_tsr("/bear/{slug}/people", get(access_view))
        .route_with_tsr("/bear/{slug}/persona", get(persona_view))
        .route_with_tsr("/bear/{slug}/stances", get(stances_view))
        .route_with_tsr("/bear/{slug}/profiles", get(stances_view))
        .route_with_tsr("/bear/{slug}/models", get(models_view).post(models_post))
        .route_with_tsr(
            "/bear/{slug}/models/provision-bifrost-key",
            post(provision_bifrost_virtual_key_action),
        )
        .route_with_tsr("/bear/{slug}/stances/{stance}", get(stance_detail_view))
        .route_with_tsr("/bear/{slug}/profiles/{stance}", get(stance_detail_view))
        .route_with_tsr(
            "/bear/{slug}/stances/{stance}/model",
            post(stance_model_post),
        )
        .route_with_tsr(
            "/bear/{slug}/profiles/{stance}/model",
            post(stance_model_post),
        )
        .route_with_tsr("/bear/{slug}/activity", get(conversations_view))
        .route_with_tsr("/bear/{slug}/conversations", get(conversations_view))
        .route_with_tsr(
            "/bear/{slug}/conversations/{conversation_id}",
            get(conversation_detail_view),
        )
        .route_with_tsr("/bear/{slug}/context", get(context_view))
        .route_with_tsr("/bear/{slug}/resources", get(policy_view))
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
            "/bear/{slug}/provision-missing-stances",
            post(provision_missing_stances_action),
        )
        .route_with_tsr(
            "/bear/{slug}/provision-missing-profiles",
            post(provision_missing_stances_action),
        )
        .route_with_tsr(
            "/bear/{slug}/provision-missing-roles",
            post(provision_missing_stances_action),
        )
}

#[derive(Debug, Deserialize)]
struct DomainQuery {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BearModelsForm {
    #[serde(default)]
    bear_default_model: String,
    #[serde(default)]
    bear_default_model_custom: String,
    #[serde(default)]
    bear_loop_control: String,
    #[serde(default)]
    chat_model: String,
    #[serde(default)]
    chat_model_custom: String,
    #[serde(default)]
    chat_loop_control: String,
    #[serde(default)]
    pair_model: String,
    #[serde(default)]
    pair_model_custom: String,
    #[serde(default)]
    pair_loop_control: String,
    #[serde(default)]
    curate_model: String,
    #[serde(default)]
    curate_model_custom: String,
    #[serde(default)]
    curate_loop_control: String,
    #[serde(default)]
    work_model: String,
    #[serde(default)]
    work_model_custom: String,
    #[serde(default)]
    work_loop_control: String,
    #[serde(default)]
    watch_model: String,
    #[serde(default)]
    watch_model_custom: String,
    #[serde(default)]
    watch_loop_control: String,
    #[serde(default)]
    bifrost_virtual_key_id: String,
    #[serde(default)]
    bifrost_virtual_key_name: String,
    #[serde(default)]
    bifrost_virtual_key_value: String,
    #[serde(default)]
    bifrost_virtual_key_clear: String,
}

#[derive(Debug, Deserialize)]
struct StanceModelForm {
    #[serde(default)]
    model: String,
    #[serde(default)]
    model_custom: String,
}

#[derive(Debug, Serialize)]
struct BearProfileModelRow {
    profile: String,
    label: String,
    configured_model: String,
    configured_model_custom: String,
    resolved_model: String,
    source: String,
    availability_status: String,
    metadata_status: String,
    configured_loop_control: String,
    resolved_loop_control: String,
    loop_control_source: String,
}

#[derive(Debug, Serialize)]
struct BifrostUsageBudgetRow {
    scope: String,
    max_limit: String,
    current_usage: String,
    remaining: String,
    reset_duration: String,
}

#[derive(Debug, Serialize)]
struct BifrostUsageModelRow {
    model: String,
    provider: String,
    total_requests: String,
    total_tokens: String,
    total_cost: String,
}

#[derive(Debug, Serialize)]
struct BifrostUsageProviderRow {
    provider: String,
    allowed_models: String,
    budget_count: usize,
}

#[derive(Debug, Serialize)]
struct BifrostUsageView {
    status: String,
    error: String,
    virtual_key_name: String,
    is_active: String,
    auth_mode: String,
    budget_rows: Vec<BifrostUsageBudgetRow>,
    model_usage_rows: Vec<BifrostUsageModelRow>,
    provider_rows: Vec<BifrostUsageProviderRow>,
    has_budgets: bool,
    has_model_usage: bool,
    has_providers: bool,
}

const MODELS_FLASH_MESSAGE_KEY: &str = "bear_models_flash_message";
const MODELS_FLASH_ERROR_KEY: &str = "bear_models_flash_error";

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
    compaction_status: String,
    compaction_event_count: i64,
    latest_compaction_at: Option<String>,
    latest_context_budget: Option<ContextBudgetReport>,
    latest_context_budget_updated_at: Option<String>,
    latest_context_budget_summary: Option<String>,
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
struct CompactionEventAdminRow {
    trigger: String,
    status: String,
    policy_version: String,
    source_group_start: Option<i32>,
    source_group_end: Option<i32>,
    diagnostic: Option<String>,
    artifact_json: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct CheckpointArtifactAdminRow {
    run_id: String,
    checkpoint_id: String,
    reason: String,
    control_level: String,
    validation_status: String,
    visibility: String,
    replay_policy: String,
    related_task_list_id: Option<String>,
    related_task_item_id: Option<String>,
    request_json: String,
    response_json: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct CompactionArtifactAdminRow {
    id: Uuid,
    artifact_kind: String,
    policy_version: String,
    trigger: String,
    source_message_start_seq: i64,
    source_message_end_seq: i64,
    source_group_start: Option<i32>,
    source_group_end: Option<i32>,
    artifact_json: String,
    superseded_by: Option<Uuid>,
    created_at: String,
}

fn context_budget_summary(report: &ContextBudgetReport) -> String {
    match report.context_window {
        Some(limit) => format!(
            "{} / {} tokens (reserve {})",
            report.estimated_total_tokens, limit, report.reserved_output_tokens
        ),
        None => format!(
            "{} tokens estimated (reserve {})",
            report.estimated_total_tokens, report.reserved_output_tokens
        ),
    }
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

pub(crate) fn bear_nav_context(bear: &den_service::bears::Bear, active: &str) -> minijinja::Value {
    context! {
        bear,
        bear_nav_active => active,
    }
}

async fn set_models_flash(session: &Session, message: &str) -> Result<(), CustomError> {
    session
        .insert(MODELS_FLASH_MESSAGE_KEY, message.to_string())
        .await
        .map_err(|err| CustomError::System(format!("could not set models flash message: {err}")))
}

async fn take_models_flash(
    session: &Session,
) -> Result<(Option<String>, Option<String>), CustomError> {
    let message = session
        .get::<String>(MODELS_FLASH_MESSAGE_KEY)
        .await
        .map_err(|err| {
            CustomError::System(format!("could not read models flash message: {err}"))
        })?;
    if message.is_some() {
        session
            .remove::<String>(MODELS_FLASH_MESSAGE_KEY)
            .await
            .map_err(|err| {
                CustomError::System(format!("could not clear models flash message: {err}"))
            })?;
    }

    let error = session
        .get::<String>(MODELS_FLASH_ERROR_KEY)
        .await
        .map_err(|err| CustomError::System(format!("could not read models flash error: {err}")))?;
    if error.is_some() {
        session
            .remove::<String>(MODELS_FLASH_ERROR_KEY)
            .await
            .map_err(|err| {
                CustomError::System(format!("could not clear models flash error: {err}"))
            })?;
    }

    Ok((message, error))
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
) -> Result<Result<(den_service::bears::Bear, bool), Redirect>, CustomError> {
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
) -> Result<Result<den_service::bears::Bear, Redirect>, CustomError> {
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

fn pretty_json(value: serde_json::Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

async fn conversation_compaction_events(
    pool: &sqlx::PgPool,
    external_conversation_id: Option<&str>,
    limit: i64,
) -> Result<Vec<CompactionEventAdminRow>, CustomError> {
    let Some(external_conversation_id) = external_conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            Option<i32>,
            Option<i32>,
            Option<String>,
            Option<serde_json::Value>,
            time::OffsetDateTime,
        ),
    >(
        r"
        SELECT trigger,
               status,
               policy_version,
               source_group_start,
               source_group_end,
               diagnostic,
               artifact,
               created_at
        FROM runtime_compaction_events
        WHERE conversation_id = $1
        ORDER BY created_at DESC
        LIMIT $2
        ",
    )
    .bind(external_conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| CustomError::Database(format!("list compaction events: {err}")))?;
    Ok(rows
        .into_iter()
        .map(
            |(
                trigger,
                status,
                policy_version,
                source_group_start,
                source_group_end,
                diagnostic,
                artifact,
                created_at,
            )| CompactionEventAdminRow {
                trigger,
                status,
                policy_version,
                source_group_start,
                source_group_end,
                diagnostic,
                artifact_json: artifact
                    .map(pretty_json)
                    .unwrap_or_else(|| "null".to_string()),
                created_at: created_at.to_string(),
            },
        )
        .collect())
}

async fn conversation_checkpoint_artifacts(
    pool: &sqlx::PgPool,
    bear_id: Uuid,
    session_id: Option<&str>,
    limit: i64,
) -> Result<Vec<CheckpointArtifactAdminRow>, CustomError> {
    let Some(session_id) = session_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query_as::<_, (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        serde_json::Value,
        Option<serde_json::Value>,
        time::OffsetDateTime,
        time::OffsetDateTime,
    )>(
        r"
        SELECT
            c.run_id,
            c.checkpoint_id,
            c.reason,
            c.control_level,
            c.validation_status,
            c.visibility,
            c.replay_policy,
            c.related_task_list_id,
            c.related_task_item_id,
            c.request,
            c.response,
            c.created_at,
            c.updated_at
        FROM bear_run_checkpoints c
        INNER JOIN turn_runs r ON r.run_id = c.run_id
        WHERE r.bear_id = $1 AND r.session_id = $2
        ORDER BY c.created_at DESC, c.checkpoint_id DESC
        LIMIT $3
        ",
    )
    .bind(bear_id)
    .bind(session_id)
    .bind(limit.max(1))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                run_id,
                checkpoint_id,
                reason,
                control_level,
                validation_status,
                visibility,
                replay_policy,
                related_task_list_id,
                related_task_item_id,
                request,
                response,
                created_at,
                updated_at,
            )| CheckpointArtifactAdminRow {
                run_id,
                checkpoint_id,
                reason,
                control_level,
                validation_status,
                visibility,
                replay_policy,
                related_task_list_id,
                related_task_item_id,
                request_json: pretty_json(request),
                response_json: response.map(pretty_json).unwrap_or_else(|| "null".to_string()),
                created_at: created_at.to_string(),
                updated_at: updated_at.to_string(),
            },
        )
        .collect())
}

async fn conversation_compaction_artifacts(
    pool: &sqlx::PgPool,
    conversation_id: Uuid,
    limit: i64,
) -> Result<Vec<CompactionArtifactAdminRow>, CustomError> {
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            String,
            i64,
            i64,
            Option<i32>,
            Option<i32>,
            serde_json::Value,
            Option<Uuid>,
            time::OffsetDateTime,
        ),
    >(
        r"
        SELECT id,
               artifact_kind,
               policy_version,
               trigger,
               source_message_start_seq,
               source_message_end_seq,
               source_group_start,
               source_group_end,
               artifact_json,
               superseded_by,
               created_at
        FROM conversation_compaction_artifacts
        WHERE conversation_id = $1
        ORDER BY created_at DESC
        LIMIT $2
        ",
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| CustomError::Database(format!("list compaction artifacts: {err}")))?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                artifact_kind,
                policy_version,
                trigger,
                source_message_start_seq,
                source_message_end_seq,
                source_group_start,
                source_group_end,
                artifact_json,
                superseded_by,
                created_at,
            )| CompactionArtifactAdminRow {
                id,
                artifact_kind,
                policy_version,
                trigger,
                source_message_start_seq,
                source_message_end_seq,
                source_group_start,
                source_group_end,
                artifact_json: pretty_json(artifact_json),
                superseded_by,
                created_at: created_at.to_string(),
            },
        )
        .collect())
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

fn manifest_for_bear(bear: &den_service::bears::Bear) -> Result<BearBundleManifest, CustomError> {
    let exported_birthdate = match bear.birthday {
        Some(date) => date.to_string(),
        None => bear
            .created_at
            .format(&Rfc3339)
            .map_err(|err| CustomError::System(format!("format Bear birthdate failed: {err}")))?
            .chars()
            .take(10)
            .collect(),
    };
    Ok(BearBundleManifest {
        format: BEAR_BUNDLE_FORMAT.to_string(),
        version: BEAR_BUNDLE_VERSION,
        bear: BearBundleIdentity {
            slug: bear.slug.clone(),
            name: bear.name.clone(),
            description: bear.description.clone(),
            birthdate: exported_birthdate,
            default_model: bear.default_model.clone(),
            tools_enabled: bear.tools_enabled.as_ref().map(|v| v.0.clone()),
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

fn bear_bundle_entry_name(entries: &[String], basename: &str) -> Result<String, CustomError> {
    if entries.iter().any(|name| name == basename) {
        return Ok(basename.to_string());
    }

    let candidates = entries
        .iter()
        .filter(|name| {
            !name.ends_with('/')
                && !name.starts_with("__MACOSX/")
                && !name.split('/').any(|part| part == "..")
                && name.rsplit('/').next() == Some(basename)
        })
        .cloned()
        .collect::<Vec<_>>();

    match candidates.as_slice() {
        [single] => Ok(single.clone()),
        [] => {
            let sample = entries
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            Err(CustomError::ValidationError(format!(
                ".bear bundle missing {basename}; entries include: {sample}"
            )))
        }
        _ => Err(CustomError::ValidationError(format!(
            ".bear bundle contains multiple {basename} entries: {}",
            candidates.join(", ")
        ))),
    }
}

fn read_bear_bundle(bytes: &[u8]) -> Result<(BearBundleManifest, Vec<u8>), CustomError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| CustomError::ValidationError(format!("invalid .bear zip: {err}")))?;
    let entries = (0..archive.len())
        .map(|idx| {
            archive
                .by_index(idx)
                .map(|file| file.name().to_string())
                .map_err(|err| {
                    CustomError::ValidationError(format!("read .bear zip entry failed: {err}"))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let manifest_name = bear_bundle_entry_name(&entries, "bear.yaml")?;
    let memory_name = bear_bundle_entry_name(&entries, "memory.sqlite")?;

    let mut manifest_yaml = String::new();
    archive
        .by_name(&manifest_name)
        .map_err(|err| CustomError::ValidationError(format!("open bear.yaml failed: {err}")))?
        .read_to_string(&mut manifest_yaml)
        .map_err(|err| CustomError::ValidationError(format!("read bear.yaml failed: {err}")))?;
    let mut memory_sqlite = Vec::new();
    archive
        .by_name(&memory_name)
        .map_err(|err| CustomError::ValidationError(format!("open memory.sqlite failed: {err}")))?
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
        sqlx::query("UPDATE bears SET birthday = $1::date, updated_at = NOW() WHERE id = $2")
            .bind(birthdate)
            .bind(bear_id)
            .execute(state.sqlx_pool())
            .await
            .map_err(|err| CustomError::ValidationError(format!("invalid Bear birthday: {err}")))?;
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
    let runtime_configured = true;
    let agent_health_rows = bear_agent_health_rows(&state, id, runtime_configured).await?;
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
    let pending_reviews: i64 = memory_stats
        .as_ref()
        .map(|s| s.pending_proposals + s.pending_observations)
        .unwrap_or(0);
    let recent_rows: Vec<(Uuid, Option<String>, String)> = sqlx::query_as(
        "SELECT id, current_title, to_char(updated_at, 'YYYY-MM-DD HH24:MI') \
         FROM conversations WHERE bear_id = $1 ORDER BY updated_at DESC LIMIT 5",
    )
    .bind(id)
    .fetch_all(state.sqlx_pool())
    .await
    .map_err(|err| CustomError::Database(format!("recent bear conversations: {err}")))?;
    let recent_conversations: Vec<serde_json::Value> = recent_rows
        .into_iter()
        .map(|(cid, title, updated)| {
            json!({
                "id": cid.to_string(),
                "title": title.unwrap_or_else(|| "Untitled conversation".to_string()),
                "updated_at": updated,
            })
        })
        .collect();
    let weekly_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT to_char(date_trunc('week', updated_at), 'YYYY-MM-DD'), COUNT(*)::bigint \
         FROM conversations WHERE bear_id = $1 \
           AND updated_at > now() - interval '8 weeks' \
         GROUP BY 1 ORDER BY 1 DESC",
    )
    .bind(id)
    .fetch_all(state.sqlx_pool())
    .await
    .map_err(|err| CustomError::Database(format!("bear activity over time: {err}")))?;
    let weekly_max = weekly_rows.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let weekly_activity: Vec<serde_json::Value> = weekly_rows
        .into_iter()
        .map(|(week, n)| {
            let pct = if weekly_max > 0 {
                (((n as f64 / weekly_max as f64) * 10.0).ceil() as i64) * 10
            } else {
                0
            };
            json!({ "week": week, "count": n, "pct": pct })
        })
        .collect();

    web::render_template(
        &state,
        "bear/settings/overview.html",
        auth_session,
        context! {
            message => query.message,
            member_count,
            native_runtime => true,
            context_profile_enabled => bear.context_profile.is_some(),
            runtime_configured,
            agent_health_rows,
            roles_ready,
            roles_error,
            memory_stats,
            legacy_import_locked => memory_stats.as_ref().map(|stats| stats.record_count > 0).unwrap_or(true),
            conversation_count,
            pending_reviews,
            recent_conversations,
            weekly_activity,
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
            ..bear_nav_context(&bear, "people"),
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

async fn stances_view(
    Path(slug): Path<String>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let runtime_configured = true;
    let agent_health_rows: Vec<BearProfileBindingHealthRow> =
        bear_agent_health_rows(&state, bear.id, runtime_configured).await?;
    web::render_template(
        &state,
        "bear/settings/stances.html",
        auth_session,
        context! {
            agent_health_rows,
            message => query.message,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "stances"),
        },
    )
    .await
}

fn profile_label(profile: BearProfile) -> &'static str {
    match profile {
        BearProfile::Chat => "Chat",
        BearProfile::Pair => "Pair",
        BearProfile::Curate => "Curate",
        BearProfile::Work => "Work",
        BearProfile::Watch => "Watch",
    }
}

fn form_profile_model(form: &BearModelsForm, profile: BearProfile) -> &str {
    match profile {
        BearProfile::Chat => &form.chat_model,
        BearProfile::Pair => &form.pair_model,
        BearProfile::Curate => &form.curate_model,
        BearProfile::Work => &form.work_model,
        BearProfile::Watch => &form.watch_model,
    }
}

fn form_profile_model_custom(form: &BearModelsForm, profile: BearProfile) -> &str {
    match profile {
        BearProfile::Chat => &form.chat_model_custom,
        BearProfile::Pair => &form.pair_model_custom,
        BearProfile::Curate => &form.curate_model_custom,
        BearProfile::Work => &form.work_model_custom,
        BearProfile::Watch => &form.watch_model_custom,
    }
}

fn form_profile_loop_control(form: &BearModelsForm, profile: BearProfile) -> &str {
    match profile {
        BearProfile::Chat => &form.chat_loop_control,
        BearProfile::Pair => &form.pair_loop_control,
        BearProfile::Curate => &form.curate_loop_control,
        BearProfile::Work => &form.work_loop_control,
        BearProfile::Watch => &form.watch_loop_control,
    }
}

fn parse_loop_control_form_value(raw: &str) -> Result<Option<AgentLoopControlLevel>, CustomError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("inherit") {
        return Ok(None);
    }
    match trimmed {
        "light" => Ok(Some(AgentLoopControlLevel::Light)),
        "standard" => Ok(Some(AgentLoopControlLevel::Standard)),
        "careful" => Ok(Some(AgentLoopControlLevel::Careful)),
        "strict" => Ok(Some(AgentLoopControlLevel::Strict)),
        other => Err(CustomError::ValidationError(format!(
            "unsupported agent loop control level `{other}`"
        ))),
    }
}

fn selected_or_custom_model<'a>(selected: &'a str, custom: &'a str) -> &'a str {
    if custom.trim().is_empty() {
        selected
    } else {
        custom
    }
}

fn is_inherit_model_value(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case("inherit")
}

fn configured_model_from_form(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || is_inherit_model_value(trimmed) {
        None
    } else {
        canonical_default_model_handle(trimmed)
    }
}

fn merge_model_options(primary: &[ModelOption], secondary: &[ModelOption]) -> Vec<ModelOption> {
    let mut merged = primary.to_vec();
    for option in secondary {
        if !merged
            .iter()
            .any(|existing| existing.handle == option.handle)
        {
            merged.push(option.clone());
        }
    }
    merged.sort_by(|a, b| a.label.cmp(&b.label));
    merged
}

fn model_available(options: &[ModelOption], raw: &str) -> bool {
    let requested = raw.trim();
    if requested.is_empty() {
        return false;
    }
    let requested_resolved = den_llm::model_registry::resolve_model_handle(requested);
    options.iter().any(|model| {
        if model.handle == requested {
            return true;
        }
        let Some(resolved) = requested_resolved else {
            return false;
        };
        resolved == model.handle
            || den_llm::model_registry::resolve_model_handle(&model.handle) == Some(resolved)
    })
}

fn model_availability_status(options: &[ModelOption], raw: &str) -> &'static str {
    if raw.trim().is_empty() {
        "unset"
    } else if model_available(options, raw) {
        "available"
    } else {
        "unavailable"
    }
}

fn model_metadata_status(raw: &str) -> &'static str {
    if raw.trim().is_empty() {
        "unknown"
    } else if den_llm::model_registry::entry_for_handle(raw).is_some() {
        "known"
    } else {
        "unknown"
    }
}

async fn model_page_rows(
    pool: &sqlx::PgPool,
    bear: &den_service::bears::Bear,
    select_options: &[ModelOption],
    availability_options: &[ModelOption],
) -> Result<Vec<BearProfileModelRow>, CustomError> {
    let settings = bears_db::list_profile_model_settings(pool, bear.id).await?;
    let bear_loop_control = bears_db::bear_agent_loop_control_setting(pool, bear.id).await?;
    let mut rows = Vec::new();
    for profile in BearProfile::ALL {
        let configured = settings
            .iter()
            .find(|row| row.profile == profile.as_str())
            .and_then(|row| row.model.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        let resolved = if configured.is_empty() {
            bear.default_model.as_deref().unwrap_or("")
        } else {
            configured
        };
        let profile_loop_control = settings
            .iter()
            .find(|row| row.profile == profile.as_str())
            .and_then(|row| row.agent_loop_control_level.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let resolved_loop_control = profile_loop_control
            .or_else(|| bear_loop_control.map(AgentLoopControlLevel::as_str))
            .unwrap_or("model default");
        let loop_control_source = if profile_loop_control.is_some() {
            "Stance override"
        } else if bear_loop_control.is_some() {
            "Bear default"
        } else {
            "Model default"
        };
        rows.push(BearProfileModelRow {
            profile: profile.as_str().to_string(),
            label: profile_label(profile).to_string(),
            configured_model: configured.to_string(),
            configured_model_custom: if configured.is_empty()
                || model_available(select_options, configured)
            {
                String::new()
            } else {
                configured.to_string()
            },
            resolved_model: resolved.to_string(),
            source: if configured.is_empty() {
                "Bear default"
            } else {
                "Stance override"
            }
            .to_string(),
            availability_status: model_availability_status(availability_options, resolved)
                .to_string(),
            metadata_status: model_metadata_status(resolved).to_string(),
            configured_loop_control: profile_loop_control.unwrap_or("").to_string(),
            resolved_loop_control: resolved_loop_control.to_string(),
            loop_control_source: loop_control_source.to_string(),
        });
    }
    Ok(rows)
}

fn display_number(value: Option<f64>) -> String {
    value
        .map(|value| {
            if value.fract().abs() < f64::EPSILON {
                format!("{}", value as i64)
            } else {
                format!("{value:.4}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string()
            }
        })
        .unwrap_or_else(|| "—".to_string())
}

fn display_money(value: Option<f64>) -> String {
    value
        .map(|value| {
            format!("${value:.4}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        })
        .unwrap_or_else(|| "—".to_string())
}

fn json_f64(value: &serde_json::Value, key: &str) -> Option<f64> {
    value.get(key).and_then(serde_json::Value::as_f64)
}

fn json_str(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("—")
        .to_string()
}

fn add_budget_rows(
    rows: &mut Vec<BifrostUsageBudgetRow>,
    model_rows: &mut Vec<BifrostUsageModelRow>,
    scope: String,
    budgets: Option<&Vec<serde_json::Value>>,
) {
    let Some(budgets) = budgets else {
        return;
    };
    for budget in budgets {
        let max_limit = json_f64(budget, "max_limit");
        let current_usage = json_f64(budget, "current_usage");
        rows.push(BifrostUsageBudgetRow {
            scope: scope.clone(),
            max_limit: display_money(max_limit),
            current_usage: display_money(current_usage),
            remaining: display_money(
                max_limit
                    .zip(current_usage)
                    .map(|(max, current)| max - current),
            ),
            reset_duration: json_str(budget, "reset_duration"),
        });
        if let Some(per_model) = budget
            .get("per_model_usage")
            .and_then(serde_json::Value::as_array)
        {
            for model in per_model {
                model_rows.push(BifrostUsageModelRow {
                    model: json_str(model, "model"),
                    provider: json_str(model, "provider"),
                    total_requests: display_number(json_f64(model, "total_requests")),
                    total_tokens: display_number(json_f64(model, "total_tokens")),
                    total_cost: display_money(json_f64(model, "total_cost")),
                });
            }
        }
    }
}

fn bifrost_usage_from_quota(
    quota: &den_service::bifrost_governance::BifrostVirtualKeyQuota,
) -> BifrostUsageView {
    let payload = &quota.payload;
    let mut budget_rows = Vec::new();
    let mut model_usage_rows = Vec::new();
    let top_level_budgets = payload.get("budgets").and_then(serde_json::Value::as_array);
    add_budget_rows(
        &mut budget_rows,
        &mut model_usage_rows,
        "Virtual key".to_string(),
        top_level_budgets,
    );

    let mut provider_rows = Vec::new();
    if let Some(providers) = payload
        .get("provider_configs")
        .and_then(serde_json::Value::as_array)
    {
        for provider in providers {
            let provider_name = json_str(provider, "provider");
            let allowed_models = provider
                .get("allowed_models")
                .and_then(serde_json::Value::as_array)
                .map(|models| {
                    models
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "—".to_string());
            let budgets = provider
                .get("budgets")
                .and_then(serde_json::Value::as_array);
            add_budget_rows(
                &mut budget_rows,
                &mut model_usage_rows,
                format!("Provider {provider_name}"),
                budgets,
            );
            provider_rows.push(BifrostUsageProviderRow {
                provider: provider_name,
                allowed_models,
                budget_count: budgets.map(Vec::len).unwrap_or(0),
            });
        }
    }

    if let Some(model_configs) = payload
        .get("model_configs")
        .and_then(serde_json::Value::as_array)
    {
        for model_config in model_configs {
            let model_name = json_str(model_config, "model_name");
            let provider = json_str(model_config, "provider");
            let scope = if provider == "—" {
                format!("Model {model_name}")
            } else {
                format!("Model {provider}/{model_name}")
            };
            add_budget_rows(
                &mut budget_rows,
                &mut model_usage_rows,
                scope,
                model_config
                    .get("budgets")
                    .and_then(serde_json::Value::as_array),
            );
        }
    }

    BifrostUsageView {
        status: "ok".to_string(),
        error: String::new(),
        virtual_key_name: json_str(payload, "virtual_key_name"),
        is_active: payload
            .get("is_active")
            .and_then(serde_json::Value::as_bool)
            .map(|value| if value { "yes" } else { "no" }.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        auth_mode: quota.auth_mode.as_str().to_string(),
        has_budgets: !budget_rows.is_empty(),
        has_model_usage: !model_usage_rows.is_empty(),
        has_providers: !provider_rows.is_empty(),
        budget_rows,
        model_usage_rows,
        provider_rows,
    }
}

fn bifrost_usage_from_management(
    details: &den_service::bifrost_governance::BifrostVirtualKeyDetails,
    rankings: Option<&serde_json::Value>,
) -> BifrostUsageView {
    let quota = den_service::bifrost_governance::BifrostVirtualKeyQuota {
        auth_mode: den_service::bifrost_governance::BifrostVirtualKeyAuthMode::XApiKey,
        payload: details.payload.clone(),
    };
    let mut view = bifrost_usage_from_quota(&quota);
    view.auth_mode = "management".to_string();
    if !details.name.trim().is_empty() {
        view.virtual_key_name.clone_from(&details.name);
    }

    if let Some(ranking_rows) = rankings
        .and_then(|value| value.get("rankings"))
        .and_then(serde_json::Value::as_array)
        .filter(|rows| !rows.is_empty())
    {
        view.model_usage_rows = ranking_rows
            .iter()
            .map(|row| BifrostUsageModelRow {
                model: json_str(row, "model"),
                provider: json_str(row, "provider"),
                total_requests: display_number(json_f64(row, "total_requests")),
                total_tokens: display_number(json_f64(row, "total_tokens")),
                total_cost: display_money(json_f64(row, "total_cost")),
            })
            .collect();
        view.has_model_usage = !view.model_usage_rows.is_empty();
    }

    view
}

async fn bifrost_usage_view_for_bear(state: &AppState, bear_id: Uuid) -> BifrostUsageView {
    let row = match bears_db::get_bear_bifrost_virtual_key(state.sqlx_pool(), bear_id).await {
        Ok(row) => row,
        Err(_) => {
            return BifrostUsageView {
                status: "error".to_string(),
                error: "Could not read the Bear's Bifrost virtual key metadata from Den storage."
                    .to_string(),
                virtual_key_name: String::new(),
                is_active: String::new(),
                auth_mode: String::new(),
                budget_rows: Vec::new(),
                model_usage_rows: Vec::new(),
                provider_rows: Vec::new(),
                has_budgets: false,
                has_model_usage: false,
                has_providers: false,
            };
        }
    };
    let Some(row) = row else {
        return BifrostUsageView {
            status: "missing".to_string(),
            error: "No Bifrost virtual key is configured for this Bear.".to_string(),
            virtual_key_name: String::new(),
            is_active: String::new(),
            auth_mode: String::new(),
            budget_rows: Vec::new(),
            model_usage_rows: Vec::new(),
            provider_rows: Vec::new(),
            has_budgets: false,
            has_model_usage: false,
            has_providers: false,
        };
    };
    let Some(virtual_key_id) = row
        .virtual_key_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return BifrostUsageView {
            status: "missing".to_string(),
            error: "No Bifrost virtual key id is configured for this Bear.".to_string(),
            virtual_key_name: row.virtual_key_name.unwrap_or_default(),
            is_active: String::new(),
            auth_mode: String::new(),
            budget_rows: Vec::new(),
            model_usage_rows: Vec::new(),
            provider_rows: Vec::new(),
            has_budgets: false,
            has_model_usage: false,
            has_providers: false,
        };
    };

    let client = den_service::bifrost_governance::BifrostGovernanceClient::new(&state.config);
    let details = match client.get_virtual_key_details_by_id(virtual_key_id).await {
        Ok(Some(details)) => details,
        Ok(None) => {
            return BifrostUsageView {
                status: "error".to_string(),
                error: format!(
                    "Bifrost management API could not find stored virtual key id {virtual_key_id}. Reprovision this Bear's virtual key."
                ),
                virtual_key_name: row.virtual_key_name.unwrap_or_default(),
                is_active: String::new(),
                auth_mode: String::new(),
                budget_rows: Vec::new(),
                model_usage_rows: Vec::new(),
                provider_rows: Vec::new(),
                has_budgets: false,
                has_model_usage: false,
                has_providers: false,
            };
        }
        Err(err) => {
            tracing::warn!(%bear_id, %virtual_key_id, error = %err, "Bifrost management virtual-key lookup failed while rendering usage");
            return BifrostUsageView {
                status: "unavailable".to_string(),
                error: "Bifrost usage details are temporarily unavailable because the Bifrost management API is not ready. Inference can still work while this panel is unavailable. Try refreshing this page shortly.".to_string(),
                virtual_key_name: row.virtual_key_name.unwrap_or_default(),
                is_active: "unknown".to_string(),
                auth_mode: "management".to_string(),
                budget_rows: Vec::new(),
                model_usage_rows: Vec::new(),
                provider_rows: Vec::new(),
                has_budgets: false,
                has_model_usage: false,
                has_providers: false,
            };
        }
    };

    let rankings = match client.get_model_usage_rankings(virtual_key_id).await {
        Ok(rankings) => Some(rankings),
        Err(err) => {
            tracing::warn!(%bear_id, %virtual_key_id, error = %err, "Bifrost model usage rankings unavailable while rendering usage");
            None
        }
    };
    bifrost_usage_from_management(&details, rankings.as_ref())
}

async fn render_models_page(
    state: AppState,
    auth_session: AuthSession,
    bear: den_service::bears::Bear,
    can_manage_bear: bool,
    message: Option<String>,
    error: Option<String>,
) -> Result<Response, CustomError> {
    let model_options =
        den_service::model_selection::list_selectable_model_options(state.sqlx_pool())
            .await
            .unwrap_or_else(|_| den_llm::model_registry::selectable_model_options());
    let (model_catalog_configured, live_model_options, models_fetch_error) =
        all_model_catalog_options_context_for_bear(&state, bear.id).await;
    let all_model_options = merge_model_options(&model_options, &live_model_options);
    let rows = model_page_rows(
        state.sqlx_pool(),
        &bear,
        &model_options,
        &live_model_options,
    )
    .await?;
    let bear_default_model = bear.default_model.as_deref().unwrap_or("");
    let bear_loop_control = bears_db::bear_agent_loop_control_setting(state.sqlx_pool(), bear.id)
        .await?
        .map(AgentLoopControlLevel::as_str)
        .unwrap_or("inherit");
    let bear_default_availability_status =
        model_availability_status(&live_model_options, bear_default_model);
    let bear_default_metadata_status = model_metadata_status(bear_default_model);
    let bifrost_virtual_key =
        bears_db::get_bear_bifrost_virtual_key(state.sqlx_pool(), bear.id).await?;
    let bifrost_usage = bifrost_usage_view_for_bear(&state, bear.id).await;
    web::render_template(
        &state,
        "bear/settings/models.html",
        auth_session,
        context! {
            model_catalog_configured,
            model_options,
            all_model_options,
            models_fetch_error,
            rows,
            bear_default_custom_model => if !bear_default_model.is_empty() && !model_available(&model_options, bear_default_model) { bear_default_model } else { "" },
            bear_loop_control,
            bear_default_availability_status,
            bear_default_metadata_status,
            bifrost_virtual_key_id => bifrost_virtual_key.as_ref().and_then(|row| row.virtual_key_id.as_deref()).unwrap_or(""),
            bifrost_virtual_key_name => bifrost_virtual_key.as_ref().and_then(|row| row.virtual_key_name.as_deref()).unwrap_or(""),
            bifrost_virtual_key_configured => bifrost_virtual_key.as_ref().map(|row| {
                row.virtual_key_value_encrypted.as_deref().map(|value| !value.trim().is_empty()).unwrap_or(false)
                    || row.virtual_key_value.as_deref().map(|value| !value.trim().is_empty()).unwrap_or(false)
            }).unwrap_or(false),
            bifrost_usage,
            message,
            error,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "identity"),
        },
    )
    .await
}

async fn models_view(
    Path(slug): Path<String>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    session: Session,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let (flash_message, flash_error) = take_models_flash(&session).await?;
    render_models_page(
        state,
        auth_session,
        bear,
        can_manage_bear,
        flash_message.or(query.message),
        flash_error.or(query.error),
    )
    .await
}

async fn models_post(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<BearModelsForm>,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let model_options =
        den_service::model_selection::list_selectable_model_options(state.sqlx_pool())
            .await
            .unwrap_or_else(|_| den_llm::model_registry::selectable_model_options());
    let (_, live_model_options, fetch_error) =
        all_model_catalog_options_context_for_bear(&state, bear.id).await;
    let validation_options = merge_model_options(&model_options, &live_model_options);
    if validation_options.is_empty() {
        let message = fetch_error
            .unwrap_or_else(|| "No Den model selection options are configured.".to_string());
        return Ok(Redirect::to(&format!(
            "/bear/{}/models?error={}",
            bear.slug,
            urlencoding::encode(&message)
        ))
        .into_response());
    }

    let default_trim =
        selected_or_custom_model(&form.bear_default_model, &form.bear_default_model_custom).trim();
    if !is_inherit_model_value(default_trim) && !model_available(&validation_options, default_trim)
    {
        return Ok(Redirect::to(&format!(
            "/bear/{}/models?error={}",
            bear.slug,
            urlencoding::encode("Choose inherit or a configured Den model selection option.")
        ))
        .into_response());
    }
    let default_model = configured_model_from_form(default_trim);
    let bear_loop_control = parse_loop_control_form_value(&form.bear_loop_control)?;

    for profile in BearProfile::ALL {
        parse_loop_control_form_value(form_profile_loop_control(&form, profile))?;
    }

    for profile in BearProfile::ALL {
        let raw = selected_or_custom_model(
            form_profile_model(&form, profile),
            form_profile_model_custom(&form, profile),
        )
        .trim();
        if !is_inherit_model_value(raw) && !model_available(&validation_options, raw) {
            let message = format!(
                "{} override must be a configured Den model selection option.",
                profile_label(profile)
            );
            return Ok(Redirect::to(&format!(
                "/bear/{}/models?error={}",
                bear.slug,
                urlencoding::encode(&message)
            ))
            .into_response());
        }
    }

    bears_db::update_bear(
        state.sqlx_pool(),
        bear.id,
        bears_db::BearParams {
            slug: bear.slug.as_str(),
            name: bear.name.as_str(),
            description: bear.description.as_str(),
            system_prompt: bear.system_prompt.as_str(),
            default_model: default_model.as_deref(),
            tools_enabled: bear.tools_enabled.clone(),
            context_profile: bear.context_profile.clone(),
        },
    )
    .await?;

    for profile in BearProfile::ALL {
        let raw = selected_or_custom_model(
            form_profile_model(&form, profile),
            form_profile_model_custom(&form, profile),
        )
        .trim();
        let model = configured_model_from_form(raw);
        bears_db::set_profile_model_setting(state.sqlx_pool(), bear.id, profile, model.as_deref())
            .await?;
        let loop_control = parse_loop_control_form_value(form_profile_loop_control(&form, profile))?;
        bears_db::set_profile_agent_loop_control_setting(
            state.sqlx_pool(),
            bear.id,
            profile,
            loop_control,
        )
        .await?;
    }

    bears_db::set_bear_agent_loop_control_setting(state.sqlx_pool(), bear.id, bear_loop_control)
        .await?;

    let clear_bifrost_key = matches!(
        form.bifrost_virtual_key_clear.trim(),
        "on" | "true" | "1" | "yes"
    );
    if clear_bifrost_key {
        bears_db::clear_bear_bifrost_virtual_key(state.sqlx_pool(), bear.id).await?;
    } else {
        let key_id = form.bifrost_virtual_key_id.trim();
        let key_name = form.bifrost_virtual_key_name.trim();
        let new_value = form.bifrost_virtual_key_value.trim();
        if new_value.is_empty() {
            bears_db::set_bear_bifrost_virtual_key_metadata(
                state.sqlx_pool(),
                bear.id,
                (!key_id.is_empty()).then_some(key_id),
                (!key_name.is_empty()).then_some(key_name),
            )
            .await?;
        } else {
            let client =
                den_service::bifrost_governance::BifrostGovernanceClient::new(&state.config);
            let validation = client.validate_virtual_key_value(new_value).await?;
            tracing::info!(
                bear_id = %bear.id,
                auth_mode = validation.auth_mode.as_str(),
                "validated manually supplied Bifrost virtual key before saving"
            );
            bears_db::set_bear_bifrost_virtual_key(
                state.sqlx_pool(),
                bear.id,
                (!key_id.is_empty()).then_some(key_id),
                (!key_name.is_empty()).then_some(key_name),
                Some(new_value),
                &state.config.den_secret_encryption_key,
            )
            .await?;
        }
    }

    Ok(Redirect::to(&format!(
        "/bear/{}/models?message={}",
        bear.slug,
        urlencoding::encode("Model and loop-control settings saved.")
    ))
    .into_response())
}

async fn provision_bifrost_virtual_key_action(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    session: Session,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let reset_usage_tracking =
        provision_bifrost_virtual_key_for_bear(&state, bear.id, &bear.slug).await?;
    let message = if reset_usage_tracking {
        "Bifrost virtual key provisioned for this Bear. The previous key with this Bear name was archived, so Bifrost usage and budget tracking start fresh for the replacement key."
    } else {
        "Bifrost virtual key provisioned for this Bear."
    };
    set_models_flash(&session, message).await?;
    Ok(Redirect::to(&format!("/bear/{}/models", bear.slug)).into_response())
}

async fn stance_detail_view(
    Path((slug, stance)): Path<(String, String)>,
    Query(query): Query<DomainQuery>,
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let role = stance
        .parse::<BearProfile>()
        .map_err(CustomError::NotFound)?;
    let role_detail = build_role_detail_view(&state, &bear, role).await?;
    web::render_template(
        &state,
        "bear/settings/stance.html",
        auth_session,
        context! {
            role_detail,
            message => query.message,
            error => query.error,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "stances"),
        },
    )
    .await
}

async fn stance_model_post(
    Path((slug, stance)): Path<(String, String)>,
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<StanceModelForm>,
) -> Result<Response, CustomError> {
    let bear = match load_session_bear_manage(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let role = BearProfile::from_str(&stance)
        .map_err(|_| CustomError::NotFound("stance not found".to_string()))?;
    let model_options =
        den_service::model_selection::list_selectable_model_options(state.sqlx_pool())
            .await
            .unwrap_or_else(|_| den_llm::model_registry::selectable_model_options());
    let (_, live_model_options, fetch_error) =
        all_model_catalog_options_context_for_bear(&state, bear.id).await;
    let validation_options = merge_model_options(&model_options, &live_model_options);
    if validation_options.is_empty() {
        let message = fetch_error
            .unwrap_or_else(|| "No Den model selection options are configured.".to_string());
        return Ok(Redirect::to(&format!(
            "/bear/{}/stances/{}?error={}",
            bear.slug,
            role.as_str(),
            urlencoding::encode(&message)
        ))
        .into_response());
    }

    let raw = selected_or_custom_model(&form.model, &form.model_custom).trim();
    if !is_inherit_model_value(raw) && !model_available(&validation_options, raw) {
        let message = format!(
            "{} model must be inherit or a configured Den model selection option.",
            profile_label(role)
        );
        return Ok(Redirect::to(&format!(
            "/bear/{}/stances/{}?error={}",
            bear.slug,
            role.as_str(),
            urlencoding::encode(&message)
        ))
        .into_response());
    }
    let model = configured_model_from_form(raw);
    bears_db::set_profile_model_setting(state.sqlx_pool(), bear.id, role, model.as_deref()).await?;
    Ok(Redirect::to(&format!(
        "/bear/{}/stances/{}?message={}",
        bear.slug,
        role.as_str(),
        urlencoding::encode("Stance model setting saved.")
    ))
    .into_response())
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
    let external_ids: Vec<String> = rows
        .iter()
        .filter_map(|c| c.external_conversation_id.clone())
        .collect();
    let compaction_stats: HashMap<String, (i64, String, time::OffsetDateTime)> =
        if external_ids.is_empty() {
            HashMap::new()
        } else {
            sqlx::query_as::<_, (String, i64, String, time::OffsetDateTime)>(
                r"
                SELECT DISTINCT ON (conversation_id)
                       conversation_id,
                       COUNT(*) OVER (PARTITION BY conversation_id)::bigint AS event_count,
                       status,
                       created_at
                FROM runtime_compaction_events
                WHERE conversation_id = ANY($1)
                ORDER BY conversation_id, created_at DESC
                ",
            )
            .bind(&external_ids)
            .fetch_all(state.sqlx_pool())
            .await
            .map_err(|err| CustomError::Database(format!("list compaction stats: {err}")))?
            .into_iter()
            .map(|(conversation_id, count, status, created_at)| {
                (conversation_id, (count, status, created_at))
            })
            .collect()
        };
    let conversations: Vec<ConversationAdminRow> = rows
        .into_iter()
        .map(|c| {
            let external_id = c
                .external_conversation_id
                .unwrap_or_else(|| "(none)".to_string());
            let stats = compaction_stats.get(&external_id);
            ConversationAdminRow {
                id: c.id,
                external_id,
                title: c
                    .current_title
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "Untitled".to_string()),
                source_session: c
                    .source_client_session_id
                    .unwrap_or_else(|| "—".to_string()),
                updated_at: c.updated_at.to_string(),
                compaction_status: stats
                    .map(|(_, status, _)| status.clone())
                    .unwrap_or_else(|| "none".to_string()),
                compaction_event_count: stats.map(|(count, _, _)| *count).unwrap_or(0),
                latest_compaction_at: stats.map(|(_, _, created_at)| created_at.to_string()),
                latest_context_budget_updated_at: c
                    .latest_context_budget_updated_at
                    .map(|value| value.to_string()),
                latest_context_budget_summary: c
                    .latest_context_budget
                    .as_ref()
                    .map(context_budget_summary),
                latest_context_budget: c.latest_context_budget,
            }
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
            ..bear_nav_context(&bear, "activity"),
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
    let compaction_events = conversation_compaction_events(
        state.sqlx_pool(),
        conv.external_conversation_id.as_deref(),
        20,
    )
    .await?;
    let compaction_artifacts =
        conversation_compaction_artifacts(state.sqlx_pool(), conversation_id, 10).await?;
    let checkpoint_artifacts = conversation_checkpoint_artifacts(
        state.sqlx_pool(),
        bear.id,
        conv.source_client_session_id.as_deref(),
        20,
    )
    .await?;
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
            compaction_events,
            compaction_artifacts,
            checkpoint_artifacts,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "activity"),
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
            ..bear_nav_context(&bear, "resources"),
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
            runtime_configured => true,
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
                "/bear/{}/people?message={}",
                bear.slug,
                urlencoding::encode("Username is required.")
            ))
            .into_response());
        }
        match user_db::get_user_by_username(state.sqlx_pool(), uname).await? {
            Some(u) => u.id,
            None => {
                return Ok(Redirect::to(&format!(
                    "/bear/{}/people?message={}",
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
        "/bear/{}/people?message={}",
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
                "/bear/{}/people?message={}",
                bear.slug,
                urlencoding::encode("Cannot remove the last bear admin.")
            ))
            .into_response());
        }
    }
    match bears_db::revoke_membership(state.sqlx_pool(), user_id, bear.id).await {
        Ok(()) => Ok(Redirect::to(&format!(
            "/bear/{}/people?message={}",
            bear.slug,
            urlencoding::encode("Access removed.")
        ))
        .into_response()),
        Err(DenError::NotFound(_)) => Ok(Redirect::to(&format!(
            "/bear/{}/people?message={}",
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
            "/bear/{}/resources?message={}",
            bear.slug,
            urlencoding::encode("Invalid web source policy form.")
        ))
        .into_response());
    }
    let scope_value = match web_policy::normalize_web_scope_value(scope_kind, &form.scope_value) {
        Ok(scope_value) => scope_value,
        Err(err) => {
            return Ok(Redirect::to(&format!(
                "/bear/{}/resources?message={}",
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
        "/bear/{}/resources?message={}",
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
        "/bear/{}/resources?message={}",
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
            "/bear/{}/resources?message={}",
            bear.slug,
            urlencoding::encode("Invalid web approval scope.")
        ))
        .into_response());
    }
    let scope_value = match web_policy::normalize_web_scope_value(scope_kind, &form.scope_value) {
        Ok(scope_value) => scope_value,
        Err(err) => {
            return Ok(Redirect::to(&format!(
                "/bear/{}/resources?message={}",
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
        "/bear/{}/resources?message={}",
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
        "/bear/{}/resources?message={}",
        bear.slug,
        urlencoding::encode("Web approval revoked.")
    ))
    .into_response())
}

async fn provision_missing_stances_action(
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
        Ok(0) => "No missing native stance bindings to provision.".to_string(),
        Ok(n) => format!("Provisioned {n} missing native stance binding(s)."),
        Err(err) => format!("Provisioning failed: {err}"),
    };
    Ok(Redirect::to(&format!(
        "/bear/{}/stances?message={}",
        bear.slug,
        urlencoding::encode(&message)
    ))
    .into_response())
}
