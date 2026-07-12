//! `session_info` / `bear_environment` orientation tool executors.
//!
//! Composes [`crate::tools::identity::BearDirectory`] (membership + user) with
//! [`EnvironmentOps`] (memory-status snapshot, adapter runtime, config flags) and
//! the pure payload builders in `payloads.rs`.

mod payloads;
mod store;

pub use payloads::{bear_environment_payload, session_info_payload};
pub use store::EnvironmentOps;

use serde_json::{json, Value};

use crate::BearProfile;

use crate::tools::{
    context::DenToolInvocationContext, identity::BearDirectory, memory::source_client_session_id,
};

async fn memory_status_for_environment(
    env: &impl EnvironmentOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
) -> Value {
    if env.uses_native_runtime() {
        return env
            .memory_status_value(context, role)
            .await
            .unwrap_or_else(|err| {
                json!({
                    "configured": true,
                    "available": false,
                    "storage": "sqlite",
                    "status": "degraded",
                    "error": err.to_string()
                })
            });
    }
    json!({
        "configured": true,
        "available": false,
        "storage": "sqlite",
        "status": "degraded",
        "error": "native runtime memory status is unavailable"
    })
}

pub async fn session_info(
    dir: &impl BearDirectory,
    env: &impl EnvironmentOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
) -> Result<Value, crate::DenError> {
    let mut context = context.clone();
    if context.workspace_roots.is_empty() {
        if let Ok(Some(adapter_runtime)) = env.fetch_adapter_environment(&context).await {
            if let Some(roots) = adapter_runtime
                .get("trusted_workspace")
                .and_then(|value| value.get("roots"))
                .and_then(Value::as_array)
            {
                context.workspace_roots = roots
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            if context.workspace_roots.is_empty() {
                if let Some(cwd) = adapter_runtime
                    .get("trusted_workspace")
                    .and_then(|value| value.get("cwd"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    context.workspace_roots.push(cwd.to_string());
                }
            }
        }
    }
    let member_count = dir.member_count(context.bear_id).await.unwrap_or(0);
    let current_user = dir.current_user(context.user_id).await.ok();
    let memory_status = memory_status_for_environment(env, &context, role).await;
    let entities = env
        .session_entities(&context, role)
        .await
        .unwrap_or_else(|err| json!({ "status": "degraded", "error": err.to_string() }));
    Ok(session_info_payload(
        &context,
        role,
        current_user.as_ref(),
        member_count,
        &memory_status,
        &entities,
    ))
}

pub async fn bear_environment(
    dir: &impl BearDirectory,
    env: &impl EnvironmentOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
) -> Result<Value, crate::DenError> {
    let member_count = dir.member_count(context.bear_id).await.unwrap_or(0);
    let current_user = dir.current_user(context.user_id).await.ok();
    let memory_status = memory_status_for_environment(env, context, role).await;
    let entities = env
        .session_entities(context, role)
        .await
        .unwrap_or_else(|err| json!({ "status": "degraded", "error": err.to_string() }));
    let adapter_runtime = match env.fetch_adapter_environment(context).await {
        Ok(Some(value)) => value,
        Ok(None) => json!({
            "status": if source_client_session_id(context).is_some() {
                "unavailable"
            } else {
                "not_applicable"
            }
        }),
        Err(err) => json!({
            "ok": false,
            "status": "degraded",
            "error": err.to_string(),
        }),
    };
    Ok(bear_environment_payload(
        context,
        role,
        current_user.as_ref(),
        member_count,
        &memory_status,
        &entities,
        &adapter_runtime,
    ))
}
