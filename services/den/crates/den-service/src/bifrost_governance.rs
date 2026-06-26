use den_core::{config::Config, DenError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct BifrostGovernanceClient {
    http: reqwest::Client,
    management_url: String,
    username: String,
    password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BifrostVirtualKeyProvisioned {
    pub id: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
enum BifrostManagementAuth {
    Bearer(String),
    Cookie(String),
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VirtualKeyResponse {
    virtual_key: VirtualKey,
}

#[derive(Debug, Deserialize)]
struct VirtualKey {
    id: String,
    name: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct CreateVirtualKeyRequest<'a> {
    name: &'a str,
    description: &'a str,
    is_active: bool,
    provider_configs: Vec<VirtualKeyProviderConfig<'a>>,
}

#[derive(Debug, Serialize)]
struct VirtualKeyProviderConfig<'a> {
    provider: &'a str,
    allowed_models: Vec<&'a str>,
    key_ids: Vec<&'a str>,
    weight: f32,
}

#[cfg(test)]
mod tests;

impl BifrostGovernanceClient {
    #[must_use]
    pub fn new(config: &Config) -> Self {
        Self {
            http: reqwest::Client::new(),
            management_url: config
                .bifrost_management_url
                .trim_end_matches('/')
                .to_string(),
            username: config.bifrost_admin_username.clone(),
            password: config.bifrost_admin_password.clone(),
        }
    }

    fn ensure_configured(&self) -> Result<(), DenError> {
        if self.management_url.is_empty() {
            return Err(DenError::ValidationError(
                "BIFROST_MANAGEMENT_URL is required to provision Bifrost virtual keys".to_string(),
            ));
        }
        if self.username.trim().is_empty() || self.password.trim().is_empty() {
            return Err(DenError::ValidationError(
                "BIFROST_ADMIN_USERNAME and BIFROST_ADMIN_PASSWORD are required to provision Bifrost virtual keys".to_string(),
            ));
        }
        Ok(())
    }

    async fn login(&self) -> Result<BifrostManagementAuth, DenError> {
        self.ensure_configured()?;
        let url = format!("{}/session/login", self.management_url);
        let response = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "username": self.username,
                "password": self.password,
            }))
            .send()
            .await
            .map_err(|err| DenError::System(format!("Bifrost management login failed: {err}")))?;
        let status = response.status();
        let cookie_header = response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        let text = response
            .text()
            .await
            .map_err(|err| DenError::System(format!("Bifrost management login body: {err}")))?;
        if !status.is_success() {
            let hint = if status == reqwest::StatusCode::FORBIDDEN
                && text.contains("Authentication is not enabled")
            {
                "; Bifrost management auth is disabled. Den cannot create virtual keys without a management session token. Ensure Bifrost runtime /api/config reports top-level auth_config.is_enabled=true; if the checked-in config is correct, reset stale /app/data/config.db and recreate bears-bifrost."
            } else {
                ""
            };
            return Err(DenError::System(format!(
                "Bifrost management login HTTP {status}: {text}{hint}"
            )));
        }
        let payload = serde_json::from_str::<LoginResponse>(&text).map_err(|err| {
            DenError::Parsing(format!(
                "Bifrost management login JSON: {err}; body: {text}"
            ))
        })?;
        if let Some(token) = payload.token.map(|token| token.trim().to_string()) {
            if !token.is_empty() {
                return Ok(BifrostManagementAuth::Bearer(token));
            }
        }
        if !cookie_header.is_empty() {
            return Ok(BifrostManagementAuth::Cookie(cookie_header));
        }
        Err(DenError::System(format!(
            "Bifrost management login succeeded but returned neither a token nor a session cookie; body: {text}"
        )))
    }

    pub async fn create_bear_virtual_key(
        &self,
        bear_id: uuid::Uuid,
        bear_slug: &str,
    ) -> Result<BifrostVirtualKeyProvisioned, DenError> {
        let auth = self.login().await?;
        let short_id = bear_id.simple().to_string();
        let short_id = &short_id[..12];
        let name = format!("bear:{bear_slug}:{short_id}");
        let description = format!("Bifrost virtual key for BEARS Bear {bear_slug} ({bear_id})");
        let request = CreateVirtualKeyRequest {
            name: &name,
            description: &description,
            is_active: true,
            provider_configs: vec![VirtualKeyProviderConfig {
                provider: "openai",
                allowed_models: vec!["*"],
                key_ids: vec!["*"],
                weight: 1.0,
            }],
        };
        let url = format!("{}/governance/virtual-keys", self.management_url);
        let mut builder = self.http.post(&url).json(&request);
        builder = match auth {
            BifrostManagementAuth::Bearer(token) => builder.bearer_auth(token),
            BifrostManagementAuth::Cookie(cookie) => {
                builder.header(reqwest::header::COOKIE, cookie)
            }
        };
        let response = builder
            .send()
            .await
            .map_err(|err| DenError::System(format!("Bifrost virtual key create failed: {err}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| DenError::System(format!("Bifrost virtual key create body: {err}")))?;
        if !status.is_success() {
            return Err(DenError::System(format!(
                "Bifrost virtual key create HTTP {status}: {text}"
            )));
        }
        let payload = serde_json::from_str::<VirtualKeyResponse>(&text).map_err(|err| {
            DenError::Parsing(format!(
                "Bifrost virtual key create JSON: {err}; body: {text}"
            ))
        })?;
        Ok(BifrostVirtualKeyProvisioned {
            id: payload.virtual_key.id,
            name: payload.virtual_key.name,
            value: payload.virtual_key.value,
        })
    }
}
