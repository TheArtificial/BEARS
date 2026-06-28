use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;

use den_core::tools::environment::EnvironmentOps;

use crate::{
    config::Config,
    core::tools::{
        memory_read::memory_status_value, memory_write::source_acp_session_id,
        session::DenToolInvocationContext,
    },
    errors::{CustomError, DenError},
};
use den_runtime::bears::BearProfile;
use den_runtime::memory::store as memory_store;
use den_runtime::memory::MemoryStoreManager;

/// Concrete [`EnvironmentOps`] over the runtime pool/config.
pub(crate) struct DenEnvironmentOps<'a> {
    pub(crate) pool: &'a PgPool,
    pub(crate) config: &'a Config,
}

impl EnvironmentOps for DenEnvironmentOps<'_> {
    fn uses_native_runtime(&self) -> bool {
        true
    }

    fn memfs_configured(&self) -> bool {
        false
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

    async fn session_entities(
        &self,
        context: &DenToolInvocationContext,
        _role: BearProfile,
    ) -> Result<Value, DenError> {
        let stores = MemoryStoreManager::new(self.config);
        let store = stores.store_for_bear(context.bear_id).await?;
        let mut human = Value::Null;
        for (handle_type, handle_value) in [
            ("den_user", Some(context.user_id.to_string())),
            ("session_human", context.username.clone()),
        ] {
            let Some(value) = handle_value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            else {
                continue;
            };
            if let Some(entity) =
                memory_store::find_entity_by_handle(&store, handle_type, value).await?
            {
                human = json!({
                    "source": handle_type,
                    "entity": entity,
                });
                break;
            }
        }

        let mut work_surface = Value::Null;
        let mut candidates: Vec<(&str, String)> = Vec::new();
        candidates.extend(
            context
                .workspace_roots
                .iter()
                .cloned()
                .map(|root| ("workspace_root", root)),
        );
        candidates.extend(
            context
                .workspace_roots
                .iter()
                .cloned()
                .map(|root| ("checkout", root)),
        );
        if let Some(target) = context.runtime_target.clone() {
            candidates.push(("checkout", target));
        }
        for (handle_type, value) in candidates {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            if let Some(entity) =
                memory_store::find_entity_by_handle(&store, handle_type, value).await?
            {
                work_surface = json!({
                    "source": handle_type,
                    "entity": entity,
                });
                break;
            }
        }

        Ok(json!({
            "status": "ok",
            "human": human,
            "work_surface": work_surface,
        }))
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
