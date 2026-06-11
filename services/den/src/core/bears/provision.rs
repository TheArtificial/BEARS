//! Create/update Letta-backed profile runtimes after Den bear rows exist.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        bifrost::BifrostClient, letta::LettaClient, memory::MemoryStoreManager,
        memory_manager_head::register_memfs_role_view,
    },
};

use super::context_composition::render_role_prompt;
use super::db as bears_db;
use super::managed_blocks::{compile_and_store_managed_config_for_bear, get_compiled_bear_config};
use super::model::{Bear, BearProfile};
use super::runtime_plan::default_runtime_plan;
use crate::errors::CustomError;

fn memfs_sidecar_url_from_env() -> String {
    std::env::var("LETTA_MEMFS_SERVICE_URL")
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string()
}

pub async fn register_role_view_if_configured(
    letta: &LettaClient,
    bear_id: Uuid,
    role: BearProfile,
    agent_id: &str,
) -> Result<(), CustomError> {
    let base = memfs_sidecar_url_from_env();
    if base.trim().is_empty() {
        return Ok(());
    }
    match register_memfs_role_view(letta.http(), &base, agent_id, bear_id, role.as_str()).await {
        Ok(Some(view)) => {
            tracing::info!(
                %bear_id,
                role = %role,
                %agent_id,
                state = %view.state,
                canonical_tip = view.canonical_tip.as_deref(),
                view_tip = view.view_tip.as_deref(),
                "MemFS role view registered"
            );
        }
        Ok(None) => {
            tracing::debug!(%bear_id, role = %role, %agent_id, "MemFS sidecar not configured; skipped role view registration");
        }
        Err(err) => {
            tracing::warn!(%bear_id, role = %role, %agent_id, error = %err, "MemFS role view registration failed");
            return Err(err);
        }
    }
    Ok(())
}

/// Provision profile runtime bindings for a new bear (native SQLite + Den loop, or Letta-backed).
pub async fn provision_bear_if_configured(
    pool: &PgPool,
    config: &Config,
    letta: &LettaClient,
    bifrost: &BifrostClient,
    bear_id: Uuid,
) -> Result<(), CustomError> {
    if config.uses_native_agent_runtime() {
        return provision_bear_native(pool, config, bear_id).await;
    }

    if !letta.is_enabled() {
        return Ok(());
    }

    let bear = bears_db::get_bear(pool, bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;

    provision_bear_profiles(pool, config, letta, bifrost, &bear).await?;
    bears_db::backfill_default_letta_agent_type(pool, bear_id, "letta_v1_agent").await?;
    bears_db::ensure_default_runtime_plan(pool, bear_id, &default_runtime_plan()).await?;
    if bear.context_profile.is_some() {
        compile_and_store_managed_config_for_bear(pool, &bear).await?;
    }
    Ok(())
}

async fn provision_bear_native(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
) -> Result<(), CustomError> {
    let summary = reconcile_bear_native(pool, config, bear_id).await?;
    if let Some(message) = summary.diagnostic_message() {
        return Err(CustomError::System(message));
    }

    tracing::info!(%bear_id, "Den-native profile runtimes provisioned for bear");
    Ok(())
}

/// Refresh Den-native profile bindings from compiled prompts and registry metadata (no Letta HTTP).
pub async fn reconcile_bear_native(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
) -> Result<crate::core::bears::sync::BearSyncSummary, CustomError> {
    let bear = bears_db::get_bear(pool, bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;

    bears_db::ensure_bear_profile_binding_rows(pool, bear_id).await?;
    bears_db::ensure_default_runtime_plan(pool, bear_id, &default_runtime_plan()).await?;

    let memory_stores = MemoryStoreManager::new(config);
    memory_stores.store_for_bear(bear_id).await?;

    if bear.context_profile.is_some() {
        compile_and_store_managed_config_for_bear(pool, &bear).await?;
    }

    let mut outcomes = Vec::new();
    for profile in BearProfile::ALL {
        outcomes.push(reconcile_one_native_profile(pool, config, &bear, profile).await?);
    }

    Ok(crate::core::bears::sync::BearSyncSummary {
        bear_id,
        outcomes,
    })
}

async fn reconcile_one_native_profile(
    pool: &PgPool,
    config: &Config,
    bear: &Bear,
    profile: BearProfile,
) -> Result<crate::core::bears::sync::BearProfileSyncOutcome, CustomError> {
    use crate::core::bears::sync::BearProfileSyncOutcome;

    let binding_id = format!("den-native:{}:{}", bear.id, profile.as_str());
    let config_hash = profile_config_hash(pool, bear, profile, config.uses_native_agent_runtime()).await?;
    let existing = bears_db::get_bear_profile_binding(pool, bear.id, profile).await?;
    let needs_refresh = match existing.as_ref() {
        None => true,
        Some(row) => {
            row.provisioning_status != "ready"
                || row.last_provisioned_version < bear.provisioning_version
                || row.config_hash.as_ref().map(|j| j.as_ref()) != Some(&config_hash)
        }
    };

    if needs_refresh {
        if let Err(err) = bears_db::mark_bear_profile_binding_ready(
            pool,
            bear.id,
            profile,
            &binding_id,
            None,
            bear.provisioning_version,
            &config_hash,
        )
        .await
        {
            let message = format!("failed to record native profile binding: {err}");
            let _ =
                bears_db::mark_bear_profile_binding_failed(pool, bear.id, profile, &message).await;
            return Ok(BearProfileSyncOutcome {
                profile: profile.as_str().to_string(),
                runtime_binding_id: Some(binding_id),
                status: "failed".to_string(),
                message: Some(message),
            });
        }
    }

    Ok(BearProfileSyncOutcome {
        profile: profile.as_str().to_string(),
        runtime_binding_id: Some(binding_id),
        status: "synced".to_string(),
        message: None,
    })
}

pub async fn provision_missing_bear_profiles_native(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
) -> Result<usize, CustomError> {
    let bear = bears_db::get_bear(pool, bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    bears_db::ensure_bear_profile_binding_rows(pool, bear_id).await?;

    let mut missing_before = 0usize;
    for profile in BearProfile::ALL {
        let existing = bears_db::get_bear_profile_binding(pool, bear_id, profile).await?;
        let ready = existing.as_ref().is_some_and(|row| {
            row.provisioning_status == "ready"
                && row.binding_id.starts_with("den-native:")
                && row.last_provisioned_version >= bear.provisioning_version
        });
        if !ready {
            missing_before += 1;
        }
    }

    if missing_before == 0 {
        return Ok(0);
    }

    let summary = reconcile_bear_native(pool, config, bear_id).await?;
    if let Some(message) = summary.diagnostic_message() {
        return Err(CustomError::System(message));
    }
    Ok(missing_before)
}

async fn provision_bear_profiles(
    pool: &PgPool,
    config: &Config,
    letta: &LettaClient,
    bifrost: &BifrostClient,
    bear: &Bear,
) -> Result<(), CustomError> {
    bears_db::ensure_bear_profile_binding_rows(pool, bear.id).await?;

    for profile in BearProfile::ALL {
        provision_bear_profile(pool, config, letta, bifrost, bear, profile).await?;
    }

    Ok(())
}

pub async fn provision_missing_bear_profiles(
    pool: &PgPool,
    config: &Config,
    letta: &LettaClient,
    bifrost: &BifrostClient,
    bear_id: Uuid,
) -> Result<usize, CustomError> {
    if !letta.is_enabled() {
        return Err(CustomError::System(
            "Letta is not configured (set LETTA_BASE_URL).".to_string(),
        ));
    }

    let bear = bears_db::get_bear(pool, bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    bears_db::ensure_bear_profile_binding_rows(pool, bear.id).await?;

    let mut provisioned = 0usize;
    for profile in BearProfile::ALL {
        let existing = bears_db::get_bear_profile_binding(pool, bear.id, profile).await?;
        let has_agent = existing
            .as_ref()
            .and_then(|row| row.letta_agent_id.as_deref())
            .is_some_and(|id| !id.trim().is_empty());
        if has_agent {
            continue;
        }
        provision_bear_profile(pool, config, letta, bifrost, &bear, profile).await?;
        provisioned += 1;
    }

    Ok(provisioned)
}

pub async fn register_existing_role_views(
    pool: &PgPool,
    letta: &LettaClient,
) -> Result<usize, CustomError> {
    let bears = bears_db::list_bears(pool).await?;
    let mut registered = 0usize;
    for bear in bears {
        let agents = bears_db::list_bear_profile_bindings(pool, bear.id).await?;
        for agent in agents {
            let role = match agent.parsed_profile() {
                Ok(role) => role,
                Err(err) => {
                    tracing::warn!(bear_id = %bear.id, profile = %agent.profile, error = %err, "skipping MemFS view registration for invalid profile");
                    continue;
                }
            };
            let Some(agent_id) = agent
                .letta_agent_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            register_role_view_if_configured(letta, bear.id, role, agent_id).await?;
            registered += 1;
        }
    }
    Ok(registered)
}

pub async fn reconcile_bear_if_configured(
    pool: &PgPool,
    config: &Config,
    letta: &LettaClient,
    bifrost: &BifrostClient,
    bear_id: Uuid,
) -> Result<crate::core::bears::sync::BearSyncSummary, CustomError> {
    if config.uses_native_agent_runtime() {
        return reconcile_bear_native(pool, config, bear_id).await;
    }

    let bear = bears_db::get_bear(pool, bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;

    if bear.context_profile.is_some() {
        compile_and_store_managed_config_for_bear(pool, &bear).await?;
    }

    if !letta.is_enabled() {
        return Ok(crate::core::bears::sync::BearSyncSummary {
            bear_id,
            outcomes: BearProfile::ALL
                .iter()
                .map(|role| crate::core::bears::sync::BearProfileSyncOutcome {
                    profile: role.as_str().to_string(),
                    runtime_binding_id: None,
                    status: "skipped_letta_disabled".to_string(),
                    message: Some("Letta is not configured (set LETTA_BASE_URL).".to_string()),
                })
                .collect(),
        });
    }
    crate::core::bears::sync::sync_all_bear_profiles_to_letta(pool, letta, bifrost, bear_id).await
}

async fn provision_bear_profile(
    pool: &PgPool,
    config: &Config,
    letta: &LettaClient,
    bifrost: &BifrostClient,
    bear: &Bear,
    profile: BearProfile,
) -> Result<(), CustomError> {
    let existing = bears_db::get_bear_profile_binding(pool, bear.id, profile).await?;
    if let Some(existing) = existing.as_ref() {
        if existing
            .letta_agent_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
        {
            let current_hash =
                profile_config_hash(pool, bear, profile, config.uses_native_agent_runtime()).await?;
            let stored_hash = existing.config_hash.as_ref().map(|j| j.as_ref());
            if existing.last_provisioned_version >= bear.provisioning_version
                && stored_hash == Some(&current_hash)
            {
                return Ok(());
            }

            // Existing Letta profile bindings are reconciled via PATCH rather than replaced.
            let summary = crate::core::bears::sync::sync_all_bear_profiles_to_letta(
                pool, letta, bifrost, bear.id,
            )
            .await?;
            if let Some(message) = summary.diagnostic_message() {
                return Err(CustomError::System(message));
            }
            return Ok(());
        }
    }

    bears_db::mark_bear_profile_binding_provisioning(pool, bear.id, profile).await?;

    let result = create_letta_profile_agent(pool, letta, bifrost, bear, profile).await;
    match result {
        Ok(agent_id) => {
            let config_hash =
                profile_config_hash(pool, bear, profile, config.uses_native_agent_runtime()).await?;
            bears_db::mark_bear_profile_binding_ready(
                pool,
                bear.id,
                profile,
                &agent_id,
                Some(&agent_id),
                bear.provisioning_version,
                &config_hash,
            )
            .await?;
            register_role_view_if_configured(letta, bear.id, profile, &agent_id).await?;
            tracing::info!(bear_id = %bear.id, runtime_binding_id = %agent_id, profile = %profile, "Letta-backed profile runtime provisioned for bear");
            Ok(())
        }
        Err(err) => {
            let message = err.to_string();
            bears_db::mark_bear_profile_binding_failed(pool, bear.id, profile, &message).await?;
            Err(err)
        }
    }
}

async fn create_letta_profile_agent(
    pool: &PgPool,
    letta: &LettaClient,
    bifrost: &BifrostClient,
    bear: &Bear,
    profile: BearProfile,
) -> Result<String, CustomError> {
    let model = bear
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CustomError::ValidationError(
                "default_model is required to provision a Letta agent (pick a model from Letta)."
                    .to_string(),
            )
        })?;

    let agent_type = bear
        .letta_agent_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("letta_v1_agent");

    let desired_tool_ids = desired_profile_tool_ids(bear, profile);
    let tool_ids = letta.filtered_tool_ids(&desired_tool_ids).await?;
    let context_window = if bifrost.is_enabled() {
        bifrost.get_model(model).await?.map(|m| m.context_window)
    } else {
        None
    };

    let name = letta_agent_name_for_profile(bear, profile);
    let prompt = profile_prompt_text(pool, bear, profile).await?;
    let tags = profile.tags_for_bear(bear.id);

    letta
        .create_agent_with_tags(crate::core::letta::LettaCreateAgentParams {
            name: name.as_str(),
            system_prompt: prompt.as_str(),
            model: Some(model),
            context_window,
            agent_type: Some(agent_type),
            tool_ids: &tool_ids,
            tags: &tags,
        })
        .await
}

pub(crate) fn letta_agent_name_for_profile(bear: &Bear, profile: BearProfile) -> String {
    // Letta validates agent names using a filesystem-safe allow-list. Keep the name readable
    // while stripping punctuation that can fail export/memfs paths (notably parentheses).
    let base = sanitize_letta_agent_name(&bear.name);
    sanitize_letta_agent_name(&format!("{base} - {}", profile.as_str()))
}

fn sanitize_letta_agent_name(name: &str) -> String {
    static UNSAFE_CHARS: OnceLock<Regex> = OnceLock::new();
    static WHITESPACE: OnceLock<Regex> = OnceLock::new();
    static HYPHENS: OnceLock<Regex> = OnceLock::new();

    let unsafe_chars = UNSAFE_CHARS.get_or_init(|| {
        Regex::new(r#"[^\p{L}\p{N} _\-' ]+"#).expect("valid Letta name sanitizer regex")
    });
    let whitespace =
        WHITESPACE.get_or_init(|| Regex::new(r#"\s+"#).expect("valid whitespace regex"));
    let hyphens = HYPHENS.get_or_init(|| Regex::new(r#"\s*-+\s*"#).expect("valid separator regex"));

    let apostrophe_normalized = name.replace(['’', '‘', '`', '´'], "'");
    let cleaned = unsafe_chars.replace_all(&apostrophe_normalized, " ");
    let cleaned = whitespace.replace_all(cleaned.trim(), " ");
    let cleaned = hyphens.replace_all(&cleaned, " - ");
    let cleaned = whitespace.replace_all(cleaned.trim(), " ");
    let cleaned = cleaned
        .trim_matches(|c: char| c == '-' || c == '_' || c == '\'')
        .trim();

    if cleaned.is_empty() {
        "Bear".to_string()
    } else {
        cleaned.to_string()
    }
}

pub(crate) fn desired_profile_tool_ids(bear: &Bear, profile: BearProfile) -> Vec<String> {
    // Until explicit per-profile tool roster configuration lands, be conservative:
    // harness-backed profiles keep the operator-selected Letta tools, while API-direct profiles receive
    // no broad operator-selected harness tools. Den/ACP tools are exposed through their own
    // controlled paths rather than by attaching every legacy Letta tool to every profile.
    match profile {
        BearProfile::Chat | BearProfile::Work => bear.letta_tool_ids.0.clone(),
        BearProfile::Pair | BearProfile::Curate | BearProfile::Watch => Vec::new(),
    }
}

pub(crate) async fn profile_prompt_text(
    pool: &PgPool,
    bear: &Bear,
    profile: BearProfile,
) -> Result<String, CustomError> {
    if bear.context_profile.is_none() {
        return render_role_prompt(bear, profile);
    }

    if let Some(compiled) = get_compiled_bear_config(pool, bear.id).await? {
        if let Some(prompt) = compiled
            .rendered_prompts_json
            .0
            .get(profile.as_str())
            .and_then(|v| v.as_str())
        {
            return Ok(prompt.to_string());
        }
    }

    let compiled = compile_and_store_managed_config_for_bear(pool, bear).await?;
    compiled
        .rendered_prompts
        .get(profile.as_str())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            CustomError::System(format!(
                "compiled managed prompt missing rendered prompt for profile {}",
                profile.as_str()
            ))
        })
}

pub(crate) async fn profile_config_hash(
    pool: &PgPool,
    bear: &Bear,
    profile: BearProfile,
    native_runtime: bool,
) -> Result<serde_json::Value, CustomError> {
    let tool_ids = if native_runtime {
        Vec::new()
    } else {
        desired_profile_tool_ids(bear, profile)
    };
    let mut payload = json!({
        "schema_version": 1,
        "role": profile.as_str(),
        "runtime_family": profile.runtime_family(),
        "bear_provisioning_version": bear.provisioning_version,
        "tool_ids": tool_ids,
        "prompt_strategy": if bear.context_profile.is_some() { "managed_compiled_context_v1" } else { "legacy_system_prompt_v1" },
        "skills": {
            "manifest_projection": "pending"
        }
    });

    if bear.context_profile.is_some() {
        let compiled = match get_compiled_bear_config(pool, bear.id).await? {
            Some(compiled) => compiled,
            None => {
                compile_and_store_managed_config_for_bear(pool, bear).await?;
                get_compiled_bear_config(pool, bear.id)
                    .await?
                    .ok_or_else(|| {
                        CustomError::System(
                            "compiled managed config missing after successful compile".to_string(),
                        )
                    })?
            }
        };

        let prompt_hash = compiled
            .rendered_prompt_hashes_json
            .0
            .get(profile.as_str())
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        payload["compiled_config_hash"] = json!(compiled.config_hash);
        payload["compiled_version"] = json!(compiled.compiled_version);
        payload["profile_prompt_hash"] = prompt_hash;
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::types::Json;
    use time::OffsetDateTime;

    fn test_bear(name: &str) -> Bear {
        Bear {
            id: Uuid::nil(),
            slug: "builder".to_string(),
            name: name.to_string(),
            description: String::new(),
            default_model: Some("openai/gpt-4o".to_string()),
            tools_enabled: None,
            letta_agent_type: None,
            letta_tool_ids: Json(Vec::new()),
            runtime_plan: None,
            context_profile: None,
            memfs_repo_path: None,
            provisioning_version: 1,
            system_prompt: String::new(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn letta_agent_name_for_profile_uses_letta_safe_profile_suffix() {
        let bear = test_bear("Builder Bear");
        assert_eq!(
            letta_agent_name_for_profile(&bear, BearProfile::Pair),
            "Builder Bear - pair"
        );
    }

    #[test]
    fn letta_agent_name_for_profile_sanitizes_filesystem_unsafe_characters() {
        let bear = test_bear("Builder/Bear: Alpha (v2) \"ops\"?");
        let name = letta_agent_name_for_profile(&bear, BearProfile::Work);
        assert_eq!(name, "Builder Bear Alpha v2 ops - work");
        assert!(name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '\'')));
    }

    #[test]
    fn letta_agent_name_for_profile_keeps_unicode_letters_and_normalizes_apostrophes() {
        let bear = test_bear("Zoë’s 建築_Bear");
        assert_eq!(
            letta_agent_name_for_profile(&bear, BearProfile::Chat),
            "Zoë's 建築_Bear - chat"
        );
    }

    #[test]
    fn letta_agent_name_for_profile_falls_back_when_base_name_has_no_safe_characters() {
        let bear = test_bear("()/?*");
        assert_eq!(
            letta_agent_name_for_profile(&bear, BearProfile::Curate),
            "Bear - curate"
        );
    }
}
