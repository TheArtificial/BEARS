use std::time::Duration;

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
    /// True when the deterministic Bear virtual-key name already existed, so Den
    /// created a replacement with a unique reprovisioning suffix. The new key has
    /// fresh Bifrost budget/usage state.
    pub reset_usage_tracking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BifrostVirtualKeyAuthMode {
    XApiKey,
    XBfVk,
    Bearer,
}

impl BifrostVirtualKeyAuthMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::XApiKey => "x-api-key",
            Self::XBfVk => "x-bf-vk",
            Self::Bearer => "bearer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BifrostVirtualKeyValidation {
    pub auth_mode: BifrostVirtualKeyAuthMode,
}

#[derive(Debug, Clone)]
pub struct BifrostVirtualKeyQuota {
    pub auth_mode: BifrostVirtualKeyAuthMode,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct BifrostVirtualKeyDetails {
    pub id: String,
    pub name: String,
    pub value: String,
    pub payload: serde_json::Value,
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
struct ListVirtualKeysResponse {
    virtual_keys: Vec<VirtualKey>,
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
    budgets: Vec<CreateBudgetRequest<'a>>,
}

#[derive(Debug, Serialize)]
struct CreateBudgetRequest<'a> {
    max_limit: f64,
    reset_duration: &'a str,
    calendar_aligned: bool,
}

#[derive(Debug, Serialize)]
struct VirtualKeyProviderConfig<'a> {
    provider: &'a str,
    allowed_models: Vec<&'a str>,
    key_ids: Vec<&'a str>,
    weight: f32,
}

#[derive(Debug, Serialize)]
struct UpdateVirtualKeyRequest<'a> {
    name: &'a str,
    description: &'a str,
    is_active: bool,
}

#[cfg(test)]
mod tests;

fn bifrost_auth_not_enabled_response(text: &str) -> bool {
    text.to_ascii_lowercase()
        .contains("authentication is not enabled")
}

fn bifrost_auth_not_enabled_error(err: &DenError) -> bool {
    bifrost_auth_not_enabled_response(&err.to_string())
}

fn bifrost_virtual_key_quota_transient_error(err: &DenError) -> bool {
    let text = err.to_string().to_ascii_lowercase();
    text.contains("virtual_key_not_found")
        || text.contains("virtual key not found")
        || text.contains("authentication is not enabled")
}

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

    async fn login_once(&self) -> Result<BifrostManagementAuth, DenError> {
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
                && bifrost_auth_not_enabled_response(&text)
            {
                "; Bifrost management auth reported not enabled. This can be transient during Bifrost auth/config-store warmup; Den retries this response before failing. If it persists, ensure services/bifrost/config.json uses governance.auth_config.is_enabled=true and Bifrost runtime /api/session/is-auth-enabled reports true."
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

    async fn login(&self) -> Result<BifrostManagementAuth, DenError> {
        let mut last_err: Option<DenError> = None;
        for attempt in 1..=4 {
            match self.login_once().await {
                Ok(auth) => {
                    if attempt > 1 {
                        tracing::warn!(
                            attempt,
                            management_url = %self.management_url,
                            "Bifrost management login succeeded after transient auth-not-enabled response"
                        );
                    }
                    return Ok(auth);
                }
                Err(err) if bifrost_auth_not_enabled_error(&err) && attempt < 4 => {
                    tracing::warn!(
                        attempt,
                        management_url = %self.management_url,
                        error = %err,
                        "Bifrost management login reported auth not enabled; retrying after short delay"
                    );
                    last_err = Some(err);
                    tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            DenError::System("Bifrost management login failed after retries".to_string())
        }))
    }

    async fn virtual_key_quota_with_mode_once(
        &self,
        value: &str,
        mode: BifrostVirtualKeyAuthMode,
    ) -> Result<BifrostVirtualKeyQuota, DenError> {
        self.ensure_configured()?;
        let value = value.trim();
        if value.is_empty() {
            return Err(DenError::ValidationError(
                "Bifrost virtual key value is empty".to_string(),
            ));
        }
        let url = format!("{}/governance/virtual-keys/quota", self.management_url);
        let builder = self.http.get(&url);
        let builder = match mode {
            BifrostVirtualKeyAuthMode::XApiKey => builder.header("x-api-key", value),
            BifrostVirtualKeyAuthMode::XBfVk => builder.header("x-bf-vk", value),
            BifrostVirtualKeyAuthMode::Bearer => builder.bearer_auth(value),
        };
        let response = builder.send().await.map_err(|err| {
            DenError::System(format!(
                "Bifrost virtual key validation request failed using {}: {err}",
                mode.as_str()
            ))
        })?;
        let status = response.status();
        let text = response.text().await.map_err(|err| {
            DenError::System(format!(
                "Bifrost virtual key validation body failed using {}: {err}",
                mode.as_str()
            ))
        })?;
        if status.is_success() {
            let payload = serde_json::from_str::<serde_json::Value>(&text).map_err(|err| {
                DenError::Parsing(format!(
                    "Bifrost virtual key quota JSON failed using {}: {err}; body: {text}",
                    mode.as_str()
                ))
            })?;
            return Ok(BifrostVirtualKeyQuota {
                auth_mode: mode,
                payload,
            });
        }
        Err(DenError::System(format!(
            "Bifrost virtual key validation HTTP {status} using {}: {text}",
            mode.as_str()
        )))
    }

    async fn virtual_key_quota_with_mode(
        &self,
        value: &str,
        mode: BifrostVirtualKeyAuthMode,
    ) -> Result<BifrostVirtualKeyQuota, DenError> {
        let mut last_err = None;
        for attempt in 1..=4 {
            match self.virtual_key_quota_with_mode_once(value, mode).await {
                Ok(quota) => {
                    if attempt > 1 {
                        tracing::warn!(
                            attempt,
                            auth_mode = mode.as_str(),
                            management_url = %self.management_url,
                            "Bifrost virtual key quota validation succeeded after transient failure"
                        );
                    }
                    return Ok(quota);
                }
                Err(err) if bifrost_virtual_key_quota_transient_error(&err) && attempt < 4 => {
                    tracing::warn!(
                        attempt,
                        auth_mode = mode.as_str(),
                        management_url = %self.management_url,
                        error = %err,
                        "Bifrost virtual key quota validation reported transient missing key/auth state; retrying"
                    );
                    last_err = Some(err);
                    tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            DenError::System(
                "Bifrost virtual key quota validation failed after retries".to_string(),
            )
        }))
    }

    pub async fn validate_virtual_key_value(
        &self,
        value: &str,
    ) -> Result<BifrostVirtualKeyValidation, DenError> {
        let quota = self
            .virtual_key_quota_with_mode(value, BifrostVirtualKeyAuthMode::XBfVk)
            .await
            .map_err(|err| {
                DenError::System(format!(
                    "Bifrost did not recognize the provisioned virtual key via x-bf-vk: {err}"
                ))
            })?;
        Ok(BifrostVirtualKeyValidation {
            auth_mode: quota.auth_mode,
        })
    }

    fn apply_management_auth(
        &self,
        builder: reqwest::RequestBuilder,
        auth: &BifrostManagementAuth,
    ) -> reqwest::RequestBuilder {
        match auth {
            BifrostManagementAuth::Bearer(token) => builder.bearer_auth(token),
            BifrostManagementAuth::Cookie(cookie) => {
                builder.header(reqwest::header::COOKIE, cookie)
            }
        }
    }

    async fn find_virtual_key_by_name(
        &self,
        auth: &BifrostManagementAuth,
        name: &str,
    ) -> Result<Option<VirtualKey>, DenError> {
        let url = format!("{}/governance/virtual-keys", self.management_url);
        let response = self
            .apply_management_auth(
                self.http
                    .get(&url)
                    .query(&[("search", name), ("limit", "100")]),
                auth,
            )
            .send()
            .await
            .map_err(|err| DenError::System(format!("Bifrost virtual key list failed: {err}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| DenError::System(format!("Bifrost virtual key list body: {err}")))?;
        if !status.is_success() {
            return Err(DenError::System(format!(
                "Bifrost virtual key list HTTP {status}: {text}"
            )));
        }
        let payload = serde_json::from_str::<ListVirtualKeysResponse>(&text).map_err(|err| {
            DenError::Parsing(format!(
                "Bifrost virtual key list JSON: {err}; body: {text}"
            ))
        })?;
        Ok(payload
            .virtual_keys
            .into_iter()
            .find(|virtual_key| virtual_key.name == name))
    }

    async fn get_virtual_key_by_id_with_auth(
        &self,
        auth: &BifrostManagementAuth,
        virtual_key_id: &str,
    ) -> Result<Option<VirtualKey>, DenError> {
        let virtual_key_id = virtual_key_id.trim();
        if virtual_key_id.is_empty() {
            return Ok(None);
        }
        let url = format!(
            "{}/governance/virtual-keys/{virtual_key_id}",
            self.management_url
        );
        let response = self
            .apply_management_auth(self.http.get(&url), auth)
            .send()
            .await
            .map_err(|err| DenError::System(format!("Bifrost virtual key get failed: {err}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| DenError::System(format!("Bifrost virtual key get body: {err}")))?;
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(DenError::System(format!(
                "Bifrost virtual key get HTTP {status}: {text}"
            )));
        }
        let payload = serde_json::from_str::<VirtualKeyResponse>(&text).map_err(|err| {
            DenError::Parsing(format!("Bifrost virtual key get JSON: {err}; body: {text}"))
        })?;
        Ok(Some(payload.virtual_key))
    }

    async fn archive_existing_virtual_key_name(
        &self,
        auth: &BifrostManagementAuth,
        virtual_key: &VirtualKey,
    ) -> Result<String, DenError> {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let archived_name = format!("{}:replaced:{}", virtual_key.name, &suffix[..8]);
        let description = format!(
            "Archived by BEARS during virtual-key reprovisioning; replaced original key named {}",
            virtual_key.name
        );
        let request = UpdateVirtualKeyRequest {
            name: &archived_name,
            description: &description,
            is_active: false,
        };
        let url = format!(
            "{}/governance/virtual-keys/{}",
            self.management_url, virtual_key.id
        );
        let response = self
            .apply_management_auth(self.http.put(&url).json(&request), auth)
            .send()
            .await
            .map_err(|err| {
                DenError::System(format!("Bifrost virtual key archive failed: {err}"))
            })?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| DenError::System(format!("Bifrost virtual key archive body: {err}")))?;
        if !status.is_success() {
            return Err(DenError::System(format!(
                "Bifrost virtual key archive HTTP {status}: {text}"
            )));
        }
        Ok(archived_name)
    }

    pub async fn get_virtual_key_details_by_id(
        &self,
        virtual_key_id: &str,
    ) -> Result<Option<BifrostVirtualKeyDetails>, DenError> {
        let auth = self.login().await?;
        let virtual_key_id = virtual_key_id.trim();
        if virtual_key_id.is_empty() {
            return Ok(None);
        }
        let url = format!(
            "{}/governance/virtual-keys/{virtual_key_id}",
            self.management_url
        );
        let response = self
            .apply_management_auth(self.http.get(&url), &auth)
            .send()
            .await
            .map_err(|err| DenError::System(format!("Bifrost virtual key get failed: {err}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| DenError::System(format!("Bifrost virtual key get body: {err}")))?;
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(DenError::System(format!(
                "Bifrost virtual key get HTTP {status}: {text}"
            )));
        }
        let payload = serde_json::from_str::<serde_json::Value>(&text).map_err(|err| {
            DenError::Parsing(format!("Bifrost virtual key get JSON: {err}; body: {text}"))
        })?;
        let virtual_key = payload.get("virtual_key").cloned().ok_or_else(|| {
            DenError::Parsing(format!(
                "Bifrost virtual key get response missing virtual_key: {text}"
            ))
        })?;
        let id = virtual_key
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(virtual_key_id)
            .to_string();
        let name = virtual_key
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let value = virtual_key
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(Some(BifrostVirtualKeyDetails {
            id,
            name,
            value,
            payload: virtual_key,
        }))
    }

    pub async fn get_virtual_key_by_id(
        &self,
        virtual_key_id: &str,
    ) -> Result<Option<BifrostVirtualKeyProvisioned>, DenError> {
        Ok(self
            .get_virtual_key_details_by_id(virtual_key_id)
            .await?
            .map(|virtual_key| BifrostVirtualKeyProvisioned {
                id: virtual_key.id,
                name: virtual_key.name,
                value: virtual_key.value,
                reset_usage_tracking: false,
            }))
    }

    async fn usage_rankings(
        &self,
        virtual_key_id: Option<&str>,
    ) -> Result<serde_json::Value, DenError> {
        let auth = self.login().await?;
        let url = format!("{}/logs/rankings", self.management_url);
        let mut query = vec![("period", "30d")];
        if let Some(virtual_key_id) = virtual_key_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            query.push(("virtual_key_ids", virtual_key_id));
        }
        let response = self
            .apply_management_auth(self.http.get(&url).query(&query), &auth)
            .send()
            .await
            .map_err(|err| {
                DenError::System(format!("Bifrost usage rankings request failed: {err}"))
            })?;
        let status = response.status();
        let text = response.text().await.map_err(|err| {
            DenError::System(format!("Bifrost usage rankings body failed: {err}"))
        })?;
        if !status.is_success() {
            return Err(DenError::System(format!(
                "Bifrost usage rankings HTTP {status}: {text}"
            )));
        }
        serde_json::from_str::<serde_json::Value>(&text).map_err(|err| {
            DenError::Parsing(format!(
                "Bifrost usage rankings JSON failed: {err}; body: {text}"
            ))
        })
    }

    pub async fn get_model_usage_rankings(
        &self,
        virtual_key_id: &str,
    ) -> Result<serde_json::Value, DenError> {
        let virtual_key_id = virtual_key_id.trim();
        if virtual_key_id.is_empty() {
            return Err(DenError::ValidationError(
                "Bifrost virtual key id is required for usage rankings".to_string(),
            ));
        }
        self.usage_rankings(Some(virtual_key_id)).await
    }

    pub async fn get_server_model_usage_rankings(&self) -> Result<serde_json::Value, DenError> {
        self.usage_rankings(None).await
    }

    pub async fn archive_virtual_key_by_id(
        &self,
        virtual_key_id: &str,
    ) -> Result<Option<String>, DenError> {
        let auth = self.login().await?;
        let Some(existing) = self
            .get_virtual_key_by_id_with_auth(&auth, virtual_key_id)
            .await?
        else {
            return Ok(None);
        };
        let archived_name = self
            .archive_existing_virtual_key_name(&auth, &existing)
            .await?;
        Ok(Some(archived_name))
    }

    pub async fn get_virtual_key_quota(
        &self,
        value: &str,
    ) -> Result<BifrostVirtualKeyQuota, DenError> {
        self.virtual_key_quota_with_mode(value, BifrostVirtualKeyAuthMode::XBfVk)
            .await
            .map_err(|err| {
                DenError::System(format!(
                    "Bifrost did not recognize the virtual key via x-bf-vk: {err}"
                ))
            })
    }

    async fn create_virtual_key_with_name(
        &self,
        auth: &BifrostManagementAuth,
        name: &str,
        description: &str,
    ) -> Result<VirtualKeyResponse, (reqwest::StatusCode, String)> {
        let request = CreateVirtualKeyRequest {
            name,
            description,
            is_active: true,
            provider_configs: vec![VirtualKeyProviderConfig {
                provider: "openai",
                allowed_models: vec!["*"],
                key_ids: vec!["*"],
                weight: 1.0,
            }],
            // A high calendar-month budget gives Bifrost a budget cycle to attach
            // usage to without acting as a practical cap for normal operation.
            // Operators can later replace this with a real Bear budget.
            budgets: vec![CreateBudgetRequest {
                max_limit: 1_000_000.0,
                reset_duration: "1M",
                calendar_aligned: true,
            }],
        };
        let url = format!("{}/governance/virtual-keys", self.management_url);
        let response = self
            .apply_management_auth(self.http.post(&url).json(&request), auth)
            .send()
            .await
            .map_err(|err| (reqwest::StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| (reqwest::StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        if !status.is_success() {
            return Err((status, text));
        }
        serde_json::from_str::<VirtualKeyResponse>(&text).map_err(|err| {
            (
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Bifrost virtual key create JSON: {err}; body: {text}"),
            )
        })
    }

    pub async fn create_bear_virtual_key(
        &self,
        bear_id: uuid::Uuid,
        bear_slug: &str,
    ) -> Result<BifrostVirtualKeyProvisioned, DenError> {
        let auth = self.login().await?;
        let short_id = bear_id.simple().to_string();
        let short_id = &short_id[..12];
        let base_name = format!("bear:{bear_slug}:{short_id}");
        let description = format!("Bifrost virtual key for BEARS Bear {bear_slug} ({bear_id})");

        let (payload, reset_usage_tracking) = match self
            .create_virtual_key_with_name(&auth, &base_name, &description)
            .await
        {
            Ok(payload) => (payload, false),
            Err((status, text))
                if status == reqwest::StatusCode::CONFLICT && text.contains("already exists") =>
            {
                let existing = self
                    .find_virtual_key_by_name(&auth, &base_name)
                    .await?
                    .ok_or_else(|| {
                        DenError::System(format!(
                            "Bifrost reported virtual key name conflict for {base_name}, but list/search did not return an exact matching key; original HTTP {status}: {text}"
                        ))
                    })?;
                let archived_name = self
                    .archive_existing_virtual_key_name(&auth, &existing)
                    .await?;
                tracing::warn!(
                    %bear_id,
                    base_name,
                    existing_virtual_key_id = %existing.id,
                    archived_name,
                    "Bifrost virtual key name already exists; archived old key and creating replacement with fresh usage/budget tracking"
                );
                let payload = self
                    .create_virtual_key_with_name(&auth, &base_name, &description)
                    .await
                    .map_err(|(retry_status, retry_text)| {
                        DenError::System(format!(
                            "Bifrost virtual key create retry after archiving existing key failed HTTP {retry_status}: {retry_text}; original HTTP {status}: {text}"
                        ))
                    })?;
                (payload, true)
            }
            Err((status, text)) => {
                return Err(DenError::System(format!(
                    "Bifrost virtual key create HTTP {status}: {text}"
                )));
            }
        };

        Ok(BifrostVirtualKeyProvisioned {
            id: payload.virtual_key.id,
            name: payload.virtual_key.name,
            value: payload.virtual_key.value,
            reset_usage_tracking,
        })
    }
}
