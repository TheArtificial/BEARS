//! Den-side HTTP client for a sandbox provider. One struct owning a
//! `reqwest::Client` (the `BifrostClient` pattern); cheap to clone.

use crate::protocol::{
    BuildImageRequest, CatalogResponse, CreateSandboxRequest, DiffResponse, ErrorBody,
    HealthResponse, ImageStoreResponse, LogsResponse, ManagedConfig, ManagedConfigStatus,
    OperationAccepted, OperationDescriptor, PrepareRustDependenciesRequest,
    PrepareRustDependenciesResponse, PublishRequest, PublishResponse, PullImageRequest,
    RemoveImageRequest, RootInspectionResponse, SandboxDescriptor, SyncRootResponse,
};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum SandboxClientError {
    #[error("sandbox provider request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("sandbox provider returned {status} ({kind}): {message}")]
    Api {
        status: u16,
        kind: String,
        message: String,
    },
}

impl SandboxClientError {
    /// Machine-readable error kind from the provider, when present.
    pub fn kind(&self) -> Option<&str> {
        match self {
            Self::Api { kind, .. } => Some(kind),
            Self::Http(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct SandboxClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl SandboxClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(600))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn health(&self) -> Result<HealthResponse, SandboxClientError> {
        self.get_json("/sandbox/v1/health").await
    }

    pub async fn create_sandbox(
        &self,
        request: &CreateSandboxRequest,
    ) -> Result<SandboxDescriptor, SandboxClientError> {
        let response = self
            .authorized(self.http.post(self.url("/sandbox/v1/sandboxes")))
            .json(request)
            .send()
            .await?;
        Self::json_or_error(response).await
    }

    pub async fn list_sandboxes(
        &self,
        state: Option<&str>,
    ) -> Result<Vec<SandboxDescriptor>, SandboxClientError> {
        let mut request = self.authorized(self.http.get(self.url("/sandbox/v1/sandboxes")));
        if let Some(state) = state {
            request = request.query(&[("state", state)]);
        }
        Self::json_or_error(request.send().await?).await
    }

    pub async fn get_sandbox(&self, id: &str) -> Result<SandboxDescriptor, SandboxClientError> {
        self.get_json(&format!("/sandbox/v1/sandboxes/{id}")).await
    }

    pub async fn logs(
        &self,
        id: &str,
        tail_bytes: Option<u64>,
    ) -> Result<LogsResponse, SandboxClientError> {
        let mut request = self.authorized(
            self.http
                .get(self.url(&format!("/sandbox/v1/sandboxes/{id}/logs"))),
        );
        if let Some(tail_bytes) = tail_bytes {
            request = request.query(&[("tail_bytes", tail_bytes)]);
        }
        Self::json_or_error(request.send().await?).await
    }

    pub async fn diff(
        &self,
        id: &str,
        max_patch_bytes: Option<u64>,
    ) -> Result<DiffResponse, SandboxClientError> {
        let mut request = self.authorized(
            self.http
                .get(self.url(&format!("/sandbox/v1/sandboxes/{id}/diff"))),
        );
        if let Some(max_patch_bytes) = max_patch_bytes {
            request = request.query(&[("max_patch_bytes", max_patch_bytes)]);
        }
        Self::json_or_error(request.send().await?).await
    }

    pub async fn destroy(
        &self,
        id: &str,
        preserve_workspace: bool,
    ) -> Result<SandboxDescriptor, SandboxClientError> {
        let request = self
            .authorized(
                self.http
                    .delete(self.url(&format!("/sandbox/v1/sandboxes/{id}"))),
            )
            .query(&[("preserve_workspace", preserve_workspace)]);
        Self::json_or_error(request.send().await?).await
    }

    /// Push the sandbox workspace's commits to its root's upstream branch.
    pub async fn publish(
        &self,
        id: &str,
        request: &PublishRequest,
    ) -> Result<PublishResponse, SandboxClientError> {
        let response = self
            .authorized(
                self.http
                    .post(self.url(&format!("/sandbox/v1/sandboxes/{id}/publish"))),
            )
            .json(request)
            .send()
            .await?;
        Self::json_or_error(response).await
    }

    /// Run the provider-owned fixed Cargo helper for an existing sandbox.
    /// This is intentionally not a generic command-execution endpoint.
    pub async fn prepare_rust_dependencies(
        &self,
        id: &str,
        request: &PrepareRustDependenciesRequest,
    ) -> Result<PrepareRustDependenciesResponse, SandboxClientError> {
        let response = self
            .authorized(
                self.http
                    .post(self.url(&format!("/sandbox/v1/sandboxes/{id}/rust-dependencies"))),
            )
            .json(request)
            .send()
            .await?;
        Self::json_or_error(response).await
    }

    /// Selectable roots and images on this provider.
    pub async fn catalog(&self) -> Result<CatalogResponse, SandboxClientError> {
        self.get_json("/sandbox/v1/catalog").await
    }

    pub async fn inspect_root(
        &self,
        name: &str,
    ) -> Result<RootInspectionResponse, SandboxClientError> {
        self.get_json(&format!("/sandbox/v1/roots/{name}")).await
    }

    pub async fn sync_root(&self, name: &str) -> Result<SyncRootResponse, SandboxClientError> {
        let request = self.authorized(
            self.http
                .post(self.url(&format!("/sandbox/v1/roots/{name}/sync"))),
        );
        Self::json_or_error(request.send().await?).await
    }

    /// Declaratively replace the provider's Den-managed surfaces + catalog.
    pub async fn put_managed_config(
        &self,
        config: &ManagedConfig,
    ) -> Result<ManagedConfigStatus, SandboxClientError> {
        let response = self
            .authorized(self.http.put(self.url("/sandbox/v1/managed-config")))
            .json(config)
            .send()
            .await?;
        Self::json_or_error(response).await
    }

    /// Non-secret summary (counts + version) of the applied managed config.
    pub async fn managed_config_status(&self) -> Result<ManagedConfigStatus, SandboxClientError> {
        self.get_json("/sandbox/v1/managed-config").await
    }

    /// Images present in the provider engine's local store.
    pub async fn list_images(&self) -> Result<ImageStoreResponse, SandboxClientError> {
        self.get_json("/sandbox/v1/images").await
    }

    /// Start a background pull of a registry reference into the engine store.
    pub async fn pull_image(
        &self,
        reference: &str,
    ) -> Result<OperationAccepted, SandboxClientError> {
        let response = self
            .authorized(self.http.post(self.url("/sandbox/v1/images/pull")))
            .json(&PullImageRequest {
                reference: reference.to_string(),
            })
            .send()
            .await?;
        Self::json_or_error(response).await
    }

    /// Start a background build of one of the shipped image variants.
    pub async fn build_image(
        &self,
        request: &BuildImageRequest,
    ) -> Result<OperationAccepted, SandboxClientError> {
        let response = self
            .authorized(self.http.post(self.url("/sandbox/v1/images/build")))
            .json(request)
            .send()
            .await?;
        Self::json_or_error(response).await
    }

    /// Remove an image from the engine store (synchronous).
    pub async fn remove_image(&self, reference: &str) -> Result<(), SandboxClientError> {
        let response = self
            .authorized(self.http.post(self.url("/sandbox/v1/images/remove")))
            .json(&RemoveImageRequest {
                reference: reference.to_string(),
            })
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body: Result<ErrorBody, _> = response.json().await;
        let (kind, message) = match body {
            Ok(body) => (body.kind, body.error),
            Err(_) => ("unknown".to_string(), "unparseable error body".to_string()),
        };
        Err(SandboxClientError::Api {
            status: status.as_u16(),
            kind,
            message,
        })
    }

    pub async fn list_operations(&self) -> Result<Vec<OperationDescriptor>, SandboxClientError> {
        self.get_json("/sandbox/v1/operations").await
    }

    pub async fn get_operation(&self, id: &str) -> Result<OperationDescriptor, SandboxClientError> {
        self.get_json(&format!("/sandbox/v1/operations/{id}")).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.token.is_empty() {
            request
        } else {
            request.bearer_auth(&self.token)
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, SandboxClientError> {
        let response = self
            .authorized(self.http.get(self.url(path)))
            .send()
            .await?;
        Self::json_or_error(response).await
    }

    async fn json_or_error<T: serde::de::DeserializeOwned>(
        response: reqwest::Response,
    ) -> Result<T, SandboxClientError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response.json().await?);
        }
        let body: Result<ErrorBody, _> = response.json().await;
        let (kind, message) = match body {
            Ok(body) => (body.kind, body.error),
            Err(_) => ("unknown".to_string(), "unparseable error body".to_string()),
        };
        Err(SandboxClientError::Api {
            status: status.as_u16(),
            kind,
            message,
        })
    }
}
