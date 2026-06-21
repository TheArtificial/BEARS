use std::time::Duration;

use serde::Deserialize;

use crate::agent_assist::ModelOption;
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
    #[allow(dead_code)]
    pub supports_responses_api: Option<bool>,
    #[allow(dead_code)]
    pub supports_vision: Option<bool>,
}

impl BifrostModelMetadata {
    pub fn to_letta_model_option(&self) -> ModelOption {
        let label = match (self.display_name.as_deref(), self.max_output_tokens) {
            (Some(name), Some(out)) => format!(
                "{} ({} ctx / {} out)",
                name,
                format_tokens(self.context_window),
                format_tokens(out)
            ),
            (Some(name), None) => format!("{} ({} ctx)", name, format_tokens(self.context_window)),
            (None, Some(out)) => format!(
                "{} ({} ctx / {} out)",
                self.handle,
                format_tokens(self.context_window),
                format_tokens(out)
            ),
            (None, None) => format!(
                "{} ({} ctx)",
                self.handle,
                format_tokens(self.context_window)
            ),
        };
        ModelOption {
            handle: self.handle.clone(),
            label,
            context_window: Some(self.context_window),
            max_output_tokens: self.max_output_tokens,
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn format_tokens(n: u32) -> String {
    if n >= 1_000_000 {
        let whole = n / 1_000_000;
        let frac = (n % 1_000_000) / 100_000;
        if frac == 0 {
            format!("{whole}M")
        } else {
            format!("{whole}.{frac}M")
        }
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

#[derive(Debug, Deserialize)]
struct BifrostModelMetadataResponse {
    models: Vec<BifrostModelMetadata>,
}

#[derive(Debug, Deserialize)]
struct BifrostLiveModelsResponse {
    data: Vec<BifrostLiveModel>,
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
}

#[derive(Debug, Deserialize)]
struct BifrostLiveTopProvider {
    context_length: Option<u64>,
    max_completion_tokens: Option<u64>,
}

impl BifrostLiveModel {
    fn into_metadata(self) -> Option<BifrostModelMetadata> {
        let handle = self.id.trim().to_string();
        if handle.is_empty() {
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
        Some(BifrostModelMetadata {
            handle,
            provider,
            model,
            display_name: self.normalized_name.or(self.name),
            context_window,
            max_output_tokens,
            enabled: true,
            supports_tools: None,
            supports_responses_api: None,
            supports_vision: None,
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
    metadata_url: String,
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
            metadata_url: config.bifrost_metadata_url.trim().to_string(),
            llm_api_url: config.llm_api_url.trim_end_matches('/').to_string(),
            api_key: config.llm_api_key.clone(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.llm_api_url.is_empty() || !self.metadata_url.is_empty()
    }

    pub async fn list_models(&self) -> Result<Vec<BifrostModelMetadata>, DenError> {
        match self.list_live_models().await {
            Ok(models) if !models.is_empty() => Ok(models),
            Ok(_) | Err(_) => self.list_sidecar_models().await,
        }
    }

    async fn list_live_models(&self) -> Result<Vec<BifrostModelMetadata>, DenError> {
        if self.llm_api_url.is_empty() {
            return Err(DenError::System(
                "Bifrost /v1 API is not configured (set LLM_API_URL or BIFROST_BASE_URL)"
                    .to_string(),
            ));
        }
        let url = format!("{}/models", self.llm_api_url);
        let mut req = self.http.get(&url);
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
        let payload: BifrostLiveModelsResponse = serde_json::from_str(&text).map_err(|e| {
            DenError::Parsing(format!("Bifrost /v1/models JSON: {e}; body: {text}"))
        })?;
        let mut models = payload
            .data
            .into_iter()
            .filter_map(BifrostLiveModel::into_metadata)
            .collect::<Vec<_>>();
        sort_models(&mut models);
        Ok(models)
    }

    async fn list_sidecar_models(&self) -> Result<Vec<BifrostModelMetadata>, DenError> {
        if self.metadata_url.is_empty() {
            return Err(DenError::System(
                "Bifrost metadata is not configured (set BIFROST_METADATA_URL)".to_string(),
            ));
        }

        let resp = self
            .http
            .get(&self.metadata_url)
            .send()
            .await
            .map_err(|e| DenError::System(format!("Bifrost model metadata request failed: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| DenError::System(format!("Bifrost model metadata response body: {e}")))?;
        if !status.is_success() {
            return Err(DenError::System(format!(
                "Bifrost model metadata HTTP {status}: {text}"
            )));
        }

        let payload: BifrostModelMetadataResponse = serde_json::from_str(&text).map_err(|e| {
            DenError::Parsing(format!("Bifrost model metadata JSON: {e}; body: {text}"))
        })?;

        let mut models: Vec<BifrostModelMetadata> = payload
            .models
            .into_iter()
            .filter(|m| m.enabled && !m.handle.trim().is_empty())
            .collect();
        sort_models(&mut models);
        Ok(models)
    }

    pub async fn get_model(&self, handle: &str) -> Result<Option<BifrostModelMetadata>, DenError> {
        let handle = handle.trim();
        if handle.is_empty() {
            return Ok(None);
        }
        Ok(self
            .list_models()
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
                    "max_output_tokens": 32768
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
    }

    #[test]
    fn formats_large_context_labels() {
        let m = BifrostModelMetadata {
            handle: "gpt-4.1".into(),
            provider: "openai".into(),
            model: "gpt-4.1".into(),
            display_name: Some("GPT-4.1".into()),
            context_window: 1_047_576,
            max_output_tokens: Some(32_768),
            enabled: true,
            supports_tools: Some(true),
            supports_responses_api: Some(true),
            supports_vision: Some(true),
        };
        let opt = m.to_letta_model_option();
        assert_eq!(opt.handle, "gpt-4.1");
        assert!(opt.label.contains("1M ctx"));
        assert_eq!(opt.context_window, Some(1_047_576));
    }
}
