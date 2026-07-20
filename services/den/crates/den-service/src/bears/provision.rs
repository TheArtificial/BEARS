//! Create/update Den-native profile runtime bindings after bear rows exist.

use den_core::config::Config;
use sqlx::PgPool;
use uuid::Uuid;

use den_memory::MemoryStoreManager;

use super::context_composition::render_role_prompt;
use super::db as bears_db;
use super::managed_blocks::{compile_and_store_managed_config_for_bear, get_compiled_bear_config};
use super::model::{Bear, BearProfile};
use super::runtime_plan::default_runtime_plan;
use den_core::DenError;

/// Provision profile runtime bindings for a new bear (Den-native SQLite + in-process loop).
pub async fn provision_bear_if_configured(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
) -> Result<(), DenError> {
    provision_bear_native(pool, config, bear_id).await
}

async fn provision_bear_native(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
) -> Result<(), DenError> {
    let summary = reconcile_bear_native(pool, config, bear_id).await?;
    if let Some(message) = summary.diagnostic_message() {
        return Err(DenError::System(message));
    }

    tracing::info!(%bear_id, "Den-native profile runtimes provisioned for bear");
    Ok(())
}

/// Refresh Den-native profile bindings from compiled prompts and registry metadata.
pub async fn reconcile_bear_native(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
) -> Result<crate::bears::sync::BearSyncSummary, DenError> {
    let bear = bears_db::get_bear(pool, bear_id)
        .await?
        .ok_or_else(|| DenError::NotFound("bear not found".to_string()))?;

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

    Ok(crate::bears::sync::BearSyncSummary { bear_id, outcomes })
}

async fn reconcile_one_native_profile(
    pool: &PgPool,
    _config: &Config,
    bear: &Bear,
    profile: BearProfile,
) -> Result<crate::bears::sync::BearProfileSyncOutcome, DenError> {
    use crate::bears::sync::{BearProfileSyncOutcome, BearProfileSyncStatus};

    let binding_id = format!("den-native:{}:{}", bear.id, profile.as_str());
    let config_hash = profile_config_hash(pool, bear, profile).await?;
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
            bear.provisioning_version,
            &config_hash,
        )
        .await
        {
            let message = format!("failed to record native profile binding: {err}");
            if let Err(mark_err) =
                bears_db::mark_bear_profile_binding_failed(pool, bear.id, profile, &message).await
            {
                tracing::warn!(
                    bear_id = %bear.id,
                    profile = %profile.as_str(),
                    error = %mark_err,
                    "failed to mark native profile binding failed after ready-write error"
                );
            }
            return Ok(BearProfileSyncOutcome {
                profile: profile.as_str().to_string(),
                runtime_binding_id: Some(binding_id),
                status: BearProfileSyncStatus::Failed,
                message: Some(message),
            });
        }
    }

    Ok(BearProfileSyncOutcome {
        profile: profile.as_str().to_string(),
        runtime_binding_id: Some(binding_id),
        status: BearProfileSyncStatus::Synced,
        message: None,
    })
}

pub async fn provision_missing_bear_profiles_native(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
) -> Result<usize, DenError> {
    provision_missing_bear_profiles(pool, config, bear_id).await
}

pub async fn provision_missing_bear_profiles(
    pool: &PgPool,
    config: &Config,
    bear_id: Uuid,
) -> Result<usize, DenError> {
    let bear = bears_db::get_bear(pool, bear_id)
        .await?
        .ok_or_else(|| DenError::NotFound("bear not found".to_string()))?;
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
        return Err(DenError::System(message));
    }
    Ok(missing_before)
}

pub async fn profile_prompt_text(
    pool: &PgPool,
    bear: &Bear,
    profile: BearProfile,
) -> Result<String, DenError> {
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
            DenError::System(format!(
                "compiled managed prompt missing rendered prompt for profile {}",
                profile.as_str()
            ))
        })
}

pub async fn profile_config_hash(
    pool: &PgPool,
    bear: &Bear,
    profile: BearProfile,
) -> Result<serde_json::Value, DenError> {
    let mut payload = serde_json::json!({
        "schema_version": 1,
        "role": profile.as_str(),
        "runtime_family": profile.runtime_family(),
        "bear_provisioning_version": bear.provisioning_version,
        "tool_ids": [],
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
                        DenError::System(
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
        payload["compiled_config_hash"] = serde_json::json!(compiled.config_hash);
        payload["compiled_version"] = serde_json::json!(compiled.compiled_version);
        payload["profile_prompt_hash"] = prompt_hash;
    }

    Ok(payload)
}
