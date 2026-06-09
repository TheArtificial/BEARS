use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{
    config::Config,
    core::{
        bears::{db as bears_db, BearProfile},
        tools::{
            memory_read::memory_status_value,
            memory_write::source_acp_session_id,
            payloads::{bear_environment_payload, session_info_payload},
            session::DenToolInvocationContext,
        },
        user,
    },
    errors::CustomError,
};

async fn memory_status_for_environment(
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearProfile,
    pool: &PgPool,
) -> Value {
    if config.uses_native_agent_runtime() {
        return memory_status_value(config, context, role, pool)
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
    if config.letta_memfs_service_url.trim().is_empty() {
        return json!({
            "configured": false,
            "available": false,
            "status": "unavailable",
            "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)"
        });
    }
    memory_status_value(config, context, role, pool)
        .await
        .unwrap_or_else(|err| {
            json!({
                "configured": !config.letta_memfs_service_url.trim().is_empty(),
                "available": false,
                "status": "degraded",
                "error": err.to_string()
            })
        })
}

pub(crate) async fn fetch_acp_adapter_environment(
    config: &Config,
    context: &DenToolInvocationContext,
) -> Result<Option<Value>, CustomError> {
    let Some(acp_session_id) = source_acp_session_id(context) else {
        return Ok(None);
    };
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|err| {
            CustomError::System(format!(
                "bear_environment adapter client build failed: {err}"
            ))
        })?;
    let url = format!(
        "{}/acp/bears/{}/sessions/{}/runtime",
        config.api_server_url.trim_end_matches('/'),
        urlencoding::encode(&context.bear_slug),
        urlencoding::encode(&acp_session_id),
    );
    let mut request = http.get(url);
    if let Some(username) = context
        .username
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.header("X-Auth-Request-Preferred-Username", username);
    }
    let response = request
        .bearer_auth(acp_session_id.as_str())
        .send()
        .await
        .map_err(|err| {
            CustomError::System(format!("bear_environment ACP runtime fetch failed: {err}"))
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|err| {
        CustomError::System(format!("bear_environment ACP runtime read failed: {err}"))
    })?;
    match status {
        StatusCode::NOT_FOUND => Ok(None),
        _ if !status.is_success() => Err(CustomError::System(format!(
            "bear_environment ACP runtime fetch failed with {}: {}",
            status,
            body.trim()
        ))),
        _ => serde_json::from_str(&body).map(Some).map_err(|err| {
            CustomError::Parsing(format!("bear_environment ACP runtime JSON: {err}"))
        }),
    }
}

pub(crate) async fn bear_environment(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearProfile,
) -> Result<Value, CustomError> {
    let member_count = match bears_db::count_bear_members(pool, context.bear_id).await {
        Ok(count) => count,
        Err(err) => {
            tracing::warn!(
                bear_id = %context.bear_id,
                user_id = context.user_id,
                error = %err,
                "bear_environment could not count Bear members; returning degraded environment payload"
            );
            0
        }
    };
    let current_user = user::user_by_id(pool, context.user_id).await.ok();
    let memory_status = memory_status_for_environment(config, context, role, pool).await;
    let adapter_runtime = match fetch_acp_adapter_environment(config, context).await {
        Ok(Some(value)) => value,
        Ok(None) => json!({
            "status": if source_acp_session_id(context).is_some() {
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
        config,
        role,
        current_user.as_ref(),
        member_count,
        memory_status,
        adapter_runtime,
    ))
}

pub(crate) async fn session_info(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearProfile,
) -> Result<Value, CustomError> {
    let member_count = match bears_db::count_bear_members(pool, context.bear_id).await {
        Ok(count) => count,
        Err(err) => {
            tracing::warn!(
                bear_id = %context.bear_id,
                user_id = context.user_id,
                error = %err,
                "session_info could not count Bear members; returning degraded orientation payload"
            );
            0
        }
    };
    let current_user = user::user_by_id(pool, context.user_id).await.ok();
    let memory_status = memory_status_for_environment(config, context, role, pool).await;
    Ok(session_info_payload(
        context,
        role,
        current_user.as_ref(),
        member_count,
        memory_status,
    ))
}
