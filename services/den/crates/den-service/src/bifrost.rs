use std::{collections::HashMap, sync::{Arc, RwLock}, time::Duration};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use den_core::{config::Config, DenError};

#[derive(Debug, Clone, Deserialize)]
pub struct BifrostModelMetadata {
    pub handle: String,
    #[allow(dead_code)]
    pub provider: String,
    #[allow(dead_code)]
    pub model: String,
    pub display_name: Option<String>,
    pub context_window: u32,
    pub max_output_tokens: Option<u32>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[allow(dead_code)]
    pub supports_tools: Option<bool>,
    pub supports_responses_api: Option<bool>,
    #[allow(dead_code)]
    pub supports_vision: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BifrostCatalogEntry {
    pub available: bool,
    pub provider: String,
    pub provider_model_id: String,
    pub gateway_handle: String,
    pub display_name: Option<String>,
    pub context_window: u32,
    pub max_output_tokens: Option<u32>,
    pub supports_tools: Option<bool>,
    pub supports_responses_api: Option<bool>,
    pub supports_vision: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BifrostCatalogSnapshot {
    pub fetched_at: Option<OffsetDateTime>,
    pub source: String,
    pub stale: bool,
    pub models: HashMap<String, BifrostCatalogEntry>,
}

impl Default for BifrostCatalogSnapshot {
    fn default() -> Self {
        Self {
            fetched_at: None,
            source: "uninitialized".to_string(),
            stale: true,
            models: HashMap::new(),
        }
    }
}

/// Canonical catalog key for a model handle.
///
/// Prefers the Den registry's canonical key. For handles unknown to the
/// registry, returns a slash-qualified handle as-is; for a bare handle, builds
/// `provider/model` when those are known (insert time) and otherwise falls back
/// to `openai/<handle>` (resolve time, where only the handle string is
/// available). The two call sites converge for registry-known and
/// slash-qualified handles; they can differ only for a registry-unknown *bare*
/// handle whose provider is not `openai`, which `/v1/models` does not produce.
pub fn canonical_catalog_key(handle: &str, provider: Option<&str>, model: Option<&str>) -> String {
    if let Some(key) = den_llm::model_registry::resolve_model_handle(handle) {
        return key.to_string();
    }
    let trimmed = handle.trim();
    if trimmed.contains('/') {
        return trimmed.to_string();
    }
    match (provider, model) {
        (Some(p), Some(m)) if !p.trim().is_empty() && !m.trim().is_empty() => {
            format!("{}/{}", p.trim(), m.trim())
        }
        _ => format!("openai/{trimmed}"),
    }
}

impl BifrostCatalogSnapshot {
    pub fn from_available_models(models: Vec<BifrostModelMetadata>) -> Self {
        let mut entries = HashMap::new();
        for model in models {
            let canonical =
                canonical_catalog_key(&model.handle, Some(&model.provider), Some(&model.model));
            entries.insert(
                canonical,
                BifrostCatalogEntry {
                    available: true,
                    provider: model.provider,
                    provider_model_id: model.model,
                    gateway_handle: model.handle,
                    display_name: model.display_name,
                    context_window: model.context_window,
                    max_output_tokens: model.max_output_tokens,
                    supports_tools: model.supports_tools,
                    supports_responses_api: model.supports_responses_api,
                    supports_vision: model.supports_vision,
                },
            );
        }
        Self {
            fetched_at: Some(OffsetDateTime::now_utc()),
            source: "v1_models".to_string(),
            stale: false,
            models: entries,
        }
    }

    pub fn resolve(&self, handle: &str) -> Option<&BifrostCatalogEntry> {
        self.models.get(&canonical_catalog_key(handle, None, None))
    }

    pub fn models_vec(&self) -> Vec<BifrostModelMetadata> {
        let mut models = self
            .models
            .iter()
            .map(|(handle, entry)| BifrostModelMetadata {
                handle: handle.clone(),
                provider: entry.provider.clone(),
                model: entry.provider_model_id.clone(),
                display_name: entry.display_name.clone(),
                context_window: entry.context_window,
                max_output_tokens: entry.max_output_tokens,
                enabled: entry.available,
                supports_tools: entry.supports_tools,
                supports_responses_api: entry.supports_responses_api,
                supports_vision: entry.supports_vision,
            })
            .collect::<Vec<_>>();
        sort_models(&mut models);
        models
    }
}

pub type BifrostCatalogStore = Arc<RwLock<BifrostCatalogSnapshot>>;

pub fn new_catalog_store() -> BifrostCatalogStore {
    Arc::new(RwLock::new(BifrostCatalogSnapshot::default()))
}

/// Spawn a background task that warms `store` immediately and then refreshes it
/// every `refresh_secs`. Failed refreshes keep the last good snapshot and flag
/// it `stale`. A `refresh_secs` of `0` warms once with no periodic refresh.
pub fn spawn_catalog_refresh(
    client: Arc<BifrostClient>,
    store: BifrostCatalogStore,
    refresh_secs: u64,
) {
    tokio::spawn(async move {
        loop {
            client.warm_model_catalog(&store).await;
            if refresh_secs == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_secs(refresh_secs)).await;
        }
    });
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct BifrostLiveModelsResponse {
    data: Vec<BifrostLiveModel>,
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BifrostLiveModel {
    id: String,
    name: Option<String>,
    normalized_name: Option<String>,
    owned_by: Option<String>,
    context_length: Option<u64>,
    max_input_tokens: Option<u64>,
    max_output_tokens: Option<u64>,
    top_provider: Option<BifrostLiveTopProvider>,
    architecture: Option<BifrostLiveArchitecture>,
    supported_parameters: Option<Vec<String>>,
    supported_methods: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct BifrostLiveArchitecture {
    input_modalities: Option<Vec<String>>,
    output_modalities: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct BifrostLiveTopProvider {
    context_length: Option<u64>,
    max_completion_tokens: Option<u64>,
}

impl BifrostLiveModel {
    fn into_metadata(self) -> Option<BifrostModelMetadata> {
        let handle = self.id.trim().to_string();
        if handle.is_empty() || den_llm::model_registry::is_routing_wildcard_model_handle(&handle) {
            return None;
        }
        let provider = handle
            .split_once('/')
            .map(|(provider, _)| provider.to_string())
            .or(self.owned_by)
            .unwrap_or_else(|| "unknown".to_string());
        let model = handle
            .split_once('/')
            .map(|(_, model)| model.to_string())
            .unwrap_or_else(|| handle.clone());
        let context_window = self
            .context_length
            .or(self.max_input_tokens)
            .or_else(|| self.top_provider.as_ref().and_then(|p| p.context_length))
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        let max_output_tokens = self
            .max_output_tokens
            .or_else(|| self.top_provider.and_then(|p| p.max_completion_tokens))
            .and_then(|n| u32::try_from(n).ok());
        let supports_tools = self.supported_parameters.as_ref().map(|params| {
            params
                .iter()
                .any(|p| matches!(p.as_str(), "tools" | "tool_choice"))
        });
        let supports_responses_api = self
            .supported_methods
            .as_ref()
            .map(|methods| methods.iter().any(|m| m.contains("response")));
        let supports_vision = self.architecture.as_ref().map(|arch| {
            let input_has_image = arch
                .input_modalities
                .as_ref()
                .map(|modalities| {
                    modalities
                        .iter()
                        .any(|m| matches!(m.as_str(), "image" | "vision"))
                })
                .unwrap_or(false);
            let output_has_image = arch
                .output_modalities
                .as_ref()
                .map(|modalities| {
                    modalities
                        .iter()
                        .any(|m| matches!(m.as_str(), "image" | "vision"))
                })
                .unwrap_or(false);
            input_has_image || output_has_image
        });
        Some(BifrostModelMetadata {
            handle,
            provider,
            model,
            display_name: self.normalized_name.or(self.name),
            context_window,
            max_output_tokens,
            enabled: true,
            supports_tools,
            supports_responses_api,
            supports_vision,
        })
    }
}

fn sort_models(models: &mut [BifrostModelMetadata]) {
    models.sort_by(|a, b| {
        a.display_name
            .as_deref()
            .unwrap_or(&a.handle)
            .cmp(b.display_name.as_deref().unwrap_or(&b.handle))
    });
}

#[derive(Clone)]
pub struct BifrostClient {
    http: reqwest::Client,
    llm_api_url: String,
    api_key: String,
}

impl BifrostClient {
    pub fn new(config: &Config) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("reqwest client");
        Self {
            http,
            llm_api_url: config.llm_api_url.trim_end_matches('/').to_string(),
            api_key: config.llm_api_key.clone(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.llm_api_url.is_empty()
    }

    /// Live Bifrost availability from `/v1/models`.
    pub async fn list_available_models(&self) -> Result<Vec<BifrostModelMetadata>, DenError> {
        if self.llm_api_url.is_empty() {
            return Err(DenError::System(
                "Bifrost /v1 API is not configured (set LLM_API_URL or BIFROST_BASE_URL)"
                    .to_string(),
            ));
        }
        let mut models = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..25 {
            let payload = self.fetch_live_models_page(page_token.as_deref()).await?;
            models.extend(
                payload
                    .data
                    .into_iter()
                    .filter_map(BifrostLiveModel::into_metadata),
            );
            page_token = payload
                .next_page_token
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty());
            if page_token.is_none() {
                break;
            }
        }
        sort_models(&mut models);
        models.dedup_by(|a, b| a.handle == b.handle);
        Ok(models)
    }

    async fn fetch_live_models_page(
        &self,
        page_token: Option<&str>,
    ) -> Result<BifrostLiveModelsResponse, DenError> {
        let url = format!("{}/models", self.llm_api_url);
        let mut req = self.http.get(&url).query(&[("page_size", "1000")]);
        if let Some(token) = page_token {
            req = req.query(&[("page_token", token)]);
        }
        if !self.api_key.trim().is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| DenError::System(format!("Bifrost /v1/models request failed: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| DenError::System(format!("Bifrost /v1/models response body: {e}")))?;
        if !status.is_success() {
            return Err(DenError::System(format!(
                "Bifrost /v1/models HTTP {status}: {text}"
            )));
        }
        serde_json::from_str(&text)
            .map_err(|e| DenError::Parsing(format!("Bifrost /v1/models JSON: {e}; body: {text}")))
    }

    pub async fn refresh_catalog_snapshot(
        &self,
        store: &BifrostCatalogStore,
    ) -> Result<BifrostCatalogSnapshot, DenError> {
        let models = self.list_available_models().await?;
        let snapshot = BifrostCatalogSnapshot::from_available_models(models);
        if let Ok(mut guard) = store.write() {
            let current_count = guard.models.len();
            let new_count = snapshot.models.len();
            if guard.fetched_at.is_some()
                && current_count >= 100
                && new_count > 0
                && new_count < 100
                && new_count * 2 < current_count
            {
                tracing::warn!(
                    current_count,
                    new_count,
                    current_fetched_at = ?guard.fetched_at,
                    "Ignoring suspiciously small Bifrost model catalog refresh; keeping last good snapshot"
                );
                guard.stale = true;
                return Ok(guard.clone());
            }
            *guard = snapshot.clone();
        }
        Ok(snapshot)
    }

    pub async fn warm_model_catalog(&self, store: &BifrostCatalogStore) {
        if !self.is_enabled() {
            return;
        }
        match self.refresh_catalog_snapshot(store).await {
            Ok(snapshot) => {
                tracing::info!(
                    count = snapshot.models.len(),
                    stale = snapshot.stale,
                    "Refreshed Bifrost model catalog snapshot"
                );
            }
            Err(err) => {
                // Keep the last good snapshot but flag it stale for operators.
                if let Ok(mut guard) = store.write() {
                    guard.stale = true;
                }
                tracing::warn!(error = %err, "Failed to refresh Bifrost model catalog snapshot");
            }
        }
    }

    pub async fn get_model(&self, handle: &str) -> Result<Option<BifrostModelMetadata>, DenError> {
        let handle = handle.trim();
        if handle.is_empty() {
            return Ok(None);
        }
        Ok(self
            .list_available_models()
            .await?
            .into_iter()
            .find(|m| m.handle == handle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_live_models_response_to_metadata() {
        let payload: BifrostLiveModelsResponse = serde_json::from_str(
            r#"{
                "data": [{
                    "id": "openai/gpt-4.1",
                    "normalized_name": "GPT-4.1",
                    "context_length": 1047576,
                    "max_output_tokens": 32768,
                    "architecture": { "input_modalities": ["text", "image"] },
                    "supported_parameters": ["tools", "temperature"],
                    "supported_methods": ["chat_completion", "responses"]
                }]
            }"#,
        )
        .expect("parse live models");
        let model = payload
            .data
            .into_iter()
            .next()
            .unwrap()
            .into_metadata()
            .unwrap();
        assert_eq!(model.handle, "openai/gpt-4.1");
        assert_eq!(model.provider, "openai");
        assert_eq!(model.model, "gpt-4.1");
        assert_eq!(model.display_name.as_deref(), Some("GPT-4.1"));
        assert_eq!(model.context_window, 1_047_576);
        assert_eq!(model.max_output_tokens, Some(32_768));
        assert_eq!(model.supports_tools, Some(true));
        assert_eq!(model.supports_responses_api, Some(true));
        assert_eq!(model.supports_vision, Some(true));
    }
}
