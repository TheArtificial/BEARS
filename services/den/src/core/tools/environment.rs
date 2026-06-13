use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::Value;
use sqlx::PgPool;

use den_tools::environment::EnvironmentOps;

use crate::{
    config::Config,
    core::{
        bears::BearProfile,
        tools::{
            memory_read::memory_status_value, memory_write::source_acp_session_id,
            session::DenToolInvocationContext,
        },
    },
    errors::{CustomError, DenError},
};

/// Concrete [`EnvironmentOps`] over the runtime pool/config.
pub(crate) struct DenEnvironmentOps<'a> {
    pub(crate) pool: &'a PgPool,
    pub(crate) config: &'a Config,
}

#[async_trait]
impl EnvironmentOps for DenEnvironmentOps<'_> {
    fn uses_native_runtime(&self) -> bool {
        self.config.uses_native_agent_runtime()
    }

    fn memfs_configured(&self) -> bool {
        !self.config.letta_memfs_service_url.trim().is_empty()
    }

    async fn memory_status_value(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
    ) -> Result<Value, DenError> {
        memory_status_value(self.config, context, role, self.pool)
            .await
            .map_err(CustomError::into_den)
    }

    async fn fetch_acp_adapter_environment(
        &self,
        context: &DenToolInvocationContext,
    ) -> Result<Option<Value>, DenError> {
        fetch_acp_adapter_environment(self.config, context)
            .await
            .map_err(CustomError::into_den)
    }
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

