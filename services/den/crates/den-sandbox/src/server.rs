//! Axum app for the sandbox provider API (`/sandbox/v1`).
//!
//! State model: an in-memory registry of sandbox records plus container
//! labels as the durable truth on this host. After a provider restart the
//! reaper adopts labeled containers back into the registry, so nothing
//! leaks invisibly. Durable *work* state (runs, results) lives on Den, not
//! here — this service can be wiped and restarted safely.

use crate::backend::container::DockerCliBackend;
use crate::backend::{Backend, BackendError, ProvisionSpec};
use crate::metrics;
use crate::policy::{validate_selection, PolicyContext, PolicyError};
use crate::proc::{run_command, CommandSpec};
use crate::protocol::{
    CatalogImage, CatalogResponse, CatalogRoot, CleanupState, CreateSandboxRequest, DiffResponse,
    ErrorBody, HealthResponse, NetworkMode, PublishRequest, RootStatus, SandboxDescriptor,
    SandboxLifecycleState, SandboxLimits, SandboxType, SandboxUsage, SyncRootResponse,
    WorkSurface,
};
use crate::recognize::recognize_work_surface;
use crate::roots::{RootsError, RootsManager};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const REAPER_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_LOG_TAIL_BYTES: u64 = 64 * 1024;
const DEFAULT_MAX_PATCH_BYTES: u64 = 512 * 1024;

/// Settings the provider needs, extracted from [`den_core::config::Config`]
/// at the composition root so this crate never reads the environment itself.
#[derive(Clone, Debug)]
pub struct SandboxServerConfig {
    /// Static bearer token protecting the API. Empty = auth disabled
    /// (same convention as `DEN_INTERNAL_TOKEN`).
    pub service_token: String,
    /// Path to the roots JSON config file, when configured.
    pub roots_config_path: Option<String>,
    /// Directory holding pristine clones and ephemeral workspaces.
    pub workspaces_dir: String,
    /// Host-side path of `workspaces_dir` when this provider runs inside a
    /// container. Sibling-sandbox bind sources are rewritten under it (the
    /// host docker daemon resolves bind sources on the host). `None` = the
    /// provider runs directly on the sandbox host.
    pub workspaces_host_dir: Option<String>,
    /// Default container image for sandboxes.
    pub default_image: String,
    /// Maximum concurrently running sandboxes.
    pub max_concurrent: usize,
    /// Per-sandbox wall-clock timeout in seconds.
    pub default_timeout_secs: u64,
    /// Cap on retained sandbox log bytes.
    pub max_log_bytes: u64,
}

impl SandboxServerConfig {
    pub fn from_config(config: &den_core::config::Config) -> Self {
        Self {
            service_token: config.sandbox_service_token.clone(),
            roots_config_path: config.sandbox_roots_config.clone(),
            workspaces_dir: config.sandbox_workspaces_dir.clone(),
            workspaces_host_dir: config.sandbox_workspaces_host_dir.clone(),
            default_image: config.sandbox_default_image.clone(),
            max_concurrent: config.sandbox_max_concurrent,
            default_timeout_secs: config.sandbox_default_timeout_secs,
            max_log_bytes: config.sandbox_max_log_bytes,
        }
    }
}

struct SandboxRecord {
    id: String,
    state: SandboxLifecycleState,
    sandbox_type: SandboxType,
    root: String,
    git_ref: Option<String>,
    work_surface: WorkSurface,
    exit_code: Option<i64>,
    usage: SandboxUsage,
    cleanup: CleanupState,
    network: NetworkMode,
    labels: BTreeMap<String, String>,
    workspace: Option<PathBuf>,
    created_at: OffsetDateTime,
    started: Instant,
    deadline: Option<Instant>,
    finished_at: Option<OffsetDateTime>,
    preserve_workspace: bool,
}

impl SandboxRecord {
    fn descriptor(&self, strength_base: &str) -> SandboxDescriptor {
        let strength_label = match self.network {
            NetworkMode::Restricted => {
                format!("{strength_base}; egress restricted to a Den-only relay")
            }
            NetworkMode::Open => format!("{strength_base}; unrestricted network egress"),
        };
        SandboxDescriptor {
            id: self.id.clone(),
            state: self.state,
            sandbox_type: self.sandbox_type,
            strength_label,
            root: self.root.clone(),
            git_ref: self.git_ref.clone(),
            work_surface: self.work_surface.clone(),
            exit_code: self.exit_code,
            usage: self.usage.clone(),
            cleanup: self.cleanup.clone(),
            network: self.network,
            labels: self.labels.clone(),
            created_at: rfc3339(self.created_at),
            finished_at: self.finished_at.map(rfc3339),
        }
    }

    fn is_active(&self) -> bool {
        matches!(
            self.state,
            SandboxLifecycleState::Provisioning | SandboxLifecycleState::Running
        )
    }
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339)
        .unwrap_or_else(|_| "invalid-timestamp".to_string())
}

pub struct ProviderState {
    config: SandboxServerConfig,
    roots: RootsManager,
    backend: Backend,
    registry: tokio::sync::Mutex<BTreeMap<String, SandboxRecord>>,
}

impl ProviderState {
    async fn active_count(&self) -> usize {
        self.registry
            .lock()
            .await
            .values()
            .filter(|record| record.is_active())
            .count()
    }
}

/// Build the provider router and spawn its background reaper. Holds no
/// database pool by construction. Must be called within a tokio runtime.
pub fn create_sandbox_app(config: SandboxServerConfig) -> Result<Router, RootsError> {
    let roots = RootsManager::load(config.roots_config_path.as_deref(), &config.workspaces_dir)?;
    let backend = Backend::DockerCli(DockerCliBackend::new(
        PathBuf::from(&config.workspaces_dir).join("env"),
        config.max_log_bytes,
    ));
    let state = Arc::new(ProviderState {
        config,
        roots,
        backend,
        registry: tokio::sync::Mutex::new(BTreeMap::new()),
    });

    tokio::spawn(reaper_loop(state.clone()));

    let open = Router::new().route("/sandbox/v1/health", get(health));
    let guarded = Router::new()
        .route("/sandbox/v1/metrics", get(metrics_text))
        .route("/sandbox/v1/sandboxes", post(create_sandbox).get(list_sandboxes))
        .route("/sandbox/v1/sandboxes/{id}", get(get_sandbox))
        .route("/sandbox/v1/sandboxes/{id}", delete(destroy_sandbox))
        .route("/sandbox/v1/sandboxes/{id}/logs", get(sandbox_logs))
        .route("/sandbox/v1/sandboxes/{id}/diff", get(sandbox_diff))
        .route("/sandbox/v1/sandboxes/{id}/publish", post(publish_sandbox))
        .route("/sandbox/v1/catalog", get(catalog))
        .route("/sandbox/v1/roots/{name}/sync", post(sync_root))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_service_token,
        ));
    Ok(open.merge(guarded).with_state(state))
}

/// Static-token guard, same convention as Den's `/internal` edge: empty
/// configured token disables auth (dev only); otherwise the exact token is
/// required as `Authorization: <token>` or `Authorization: Bearer <token>`.
async fn require_service_token(
    State(state): State<Arc<ProviderState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let expected = state.config.service_token.trim();
    if expected.is_empty() {
        return next.run(request).await;
    }
    let provided = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.strip_prefix("Bearer ").unwrap_or(value).trim());
    if provided == Some(expected) {
        next.run(request).await
    } else {
        error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid sandbox service token",
        )
    }
}

fn error_response(status: StatusCode, kind: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            kind: kind.to_string(),
            error: message.into(),
        }),
    )
        .into_response()
}

fn policy_error_response(err: &PolicyError) -> Response {
    let (status, kind) = match err {
        PolicyError::UnimplementedType { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "unimplemented_type"),
        PolicyError::ReadonlyWriteConflict => (StatusCode::UNPROCESSABLE_ENTITY, "readonly_write_conflict"),
        PolicyError::RuntimeUnavailable { .. } => (StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable"),
        PolicyError::QueueFull { .. } => (StatusCode::TOO_MANY_REQUESTS, "queue_full"),
        PolicyError::UnknownRoot { .. } => (StatusCode::NOT_FOUND, "unknown_root"),
    };
    error_response(status, kind, err.to_string())
}

fn roots_error_response(err: &RootsError) -> Response {
    match err {
        RootsError::UnknownRoot(_) => error_response(StatusCode::NOT_FOUND, "unknown_root", err.to_string()),
        RootsError::UnknownImage { .. } => {
            error_response(StatusCode::UNPROCESSABLE_ENTITY, "unknown_image", err.to_string())
        }
        RootsError::PublishUnsupported { .. } => {
            error_response(StatusCode::UNPROCESSABLE_ENTITY, "publish_unsupported", err.to_string())
        }
        RootsError::DefaultRefRefused { .. } => {
            error_response(StatusCode::UNPROCESSABLE_ENTITY, "default_ref_refused", err.to_string())
        }
        RootsError::Git { .. } => error_response(StatusCode::CONFLICT, "root_sync_failed", err.to_string()),
        _ => error_response(StatusCode::INTERNAL_SERVER_ERROR, "roots_error", err.to_string()),
    }
}

fn backend_error_response(err: &BackendError) -> Response {
    match err {
        BackendError::RuntimeUnavailable(_) => {
            error_response(StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable", err.to_string())
        }
        _ => error_response(StatusCode::BAD_GATEWAY, "backend_error", err.to_string()),
    }
}

async fn health(State(state): State<Arc<ProviderState>>) -> Json<HealthResponse> {
    let backend_available = state.backend.probe().await;
    let roots = state
        .roots
        .names()
        .into_iter()
        .filter_map(|name| state.roots.get(&name).ok().map(|root| (name, root)))
        .map(|(name, root)| {
            let (ok, detail) = state.roots.status(root);
            RootStatus {
                name,
                has_upstream: root.upstream.is_some(),
                ok,
                detail,
            }
        })
        .collect();
    let active_sandboxes = state.active_count().await;
    Json(HealthResponse {
        ok: true,
        backend_available,
        active_sandboxes,
        roots,
    })
}

async fn metrics_text(State(state): State<Arc<ProviderState>>) -> Response {
    let active = state.active_count().await;
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        metrics::render_prometheus_text(active),
    )
        .into_response()
}

async fn create_sandbox(
    State(state): State<Arc<ProviderState>>,
    Json(request): Json<CreateSandboxRequest>,
) -> Response {
    // Root and image resolve before anything is reserved: bad names are
    // deterministic rejections regardless of backend availability.
    let root = match state.roots.get(&request.root) {
        Ok(root) => root.clone(),
        Err(err) => return roots_error_response(&err),
    };
    let image = match state
        .roots
        .resolve_image(request.image.as_deref(), &root, &state.config.default_image)
    {
        Ok(image) => image,
        Err(err) => return roots_error_response(&err),
    };
    if image.trim().is_empty() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_image",
            "no image in request, no catalog default, and no SANDBOX_IMAGE configured",
        );
    }

    let backend_available = state.backend.probe().await;
    let sandbox_id = uuid::Uuid::new_v4().simple().to_string();

    // Reserve a registry slot under the lock so concurrent creates cannot
    // exceed max_concurrent between check and insert.
    {
        let mut registry = state.registry.lock().await;
        let active = registry.values().filter(|r| r.is_active()).count();
        let ctx = PolicyContext {
            backend_available,
            active_sandboxes: active,
            max_concurrent: state.config.max_concurrent,
            root_known: state.roots.get(&request.root).is_ok(),
        };
        if let Err(err) = validate_selection(&request, &ctx) {
            return policy_error_response(&err);
        }
        let timeout_secs = effective_timeout_secs(&request.limits, state.config.default_timeout_secs);
        registry.insert(
            sandbox_id.clone(),
            SandboxRecord {
                id: sandbox_id.clone(),
                state: SandboxLifecycleState::Provisioning,
                sandbox_type: request.sandbox_type,
                root: request.root.clone(),
                git_ref: request.git_ref.clone(),
                work_surface: WorkSurface::default(),
                exit_code: None,
                usage: SandboxUsage::default(),
                cleanup: CleanupState::Pending,
                network: request.network,
                labels: request.labels.clone(),
                workspace: None,
                created_at: OffsetDateTime::now_utc(),
                started: Instant::now(),
                deadline: Some(Instant::now() + Duration::from_secs(timeout_secs)),
                finished_at: None,
                preserve_workspace: false,
            },
        );
    }

    match provision(&state, &sandbox_id, &request, &root, image).await {
        Ok(()) => {
            metrics::sandbox_provisioned();
            let registry = state.registry.lock().await;
            let record = registry.get(&sandbox_id).expect("record just inserted");
            (
                StatusCode::CREATED,
                Json(record.descriptor(state.backend.strength_label())),
            )
                .into_response()
        }
        Err(response) => {
            metrics::sandbox_failed();
            let workspace = {
                let mut registry = state.registry.lock().await;
                if let Some(record) = registry.get_mut(&sandbox_id) {
                    record.state = SandboxLifecycleState::Failed;
                    record.finished_at = Some(OffsetDateTime::now_utc());
                    record.workspace.clone()
                } else {
                    None
                }
            };
            if workspace.is_some() {
                match state.roots.remove_workspace(&sandbox_id).await {
                    Ok(()) => mark_cleanup(&state, &sandbox_id, CleanupState::Done).await,
                    Err(e) => {
                        metrics::cleanup_failure();
                        mark_cleanup(
                            &state,
                            &sandbox_id,
                            CleanupState::Failed {
                                reason: e.to_string(),
                            },
                        )
                        .await;
                    }
                }
            }
            response
        }
    }
}

fn effective_timeout_secs(limits: &SandboxLimits, default_secs: u64) -> u64 {
    if limits.timeout_secs == 0 {
        default_secs
    } else {
        limits.timeout_secs.min(default_secs.max(limits.timeout_secs))
    }
}

/// The slow part of creation, after the slot is reserved. Any error leaves the
/// record marked Failed by the caller.
async fn provision(
    state: &Arc<ProviderState>,
    sandbox_id: &str,
    request: &CreateSandboxRequest,
    root: &crate::roots::SyncableRoot,
    image: String,
) -> Result<(), Response> {
    if root.upstream.is_some() {
        if let Err(err) = state.roots.sync_root(root).await {
            return Err(roots_error_response(&err));
        }
    }

    let workspace = state
        .roots
        .provision_workspace(root, request.git_ref.as_deref(), sandbox_id)
        .await
        .map_err(|err| roots_error_response(&err))?;
    {
        let mut registry = state.registry.lock().await;
        if let Some(record) = registry.get_mut(sandbox_id) {
            record.workspace = Some(workspace.clone());
        }
    }

    let work_surface = recognize_work_surface(&workspace).await;

    let workspace_bind_source =
        host_bind_source(&workspace, &state.config.workspaces_dir, state.config.workspaces_host_dir.as_deref())
            .map_err(|detail| error_response(StatusCode::INTERNAL_SERVER_ERROR, "bind_source", detail))?;
    let spec = ProvisionSpec {
        id: sandbox_id.to_string(),
        workspace,
        workspace_bind_source,
        image,
        env: request.env.clone(),
        network: request.network,
        memory_mb: request.limits.memory_mb,
        cpus: request.limits.cpus,
        pids: request.limits.pids,
        labels: request.labels.clone(),
    };
    state
        .backend
        .provision(&spec)
        .await
        .map_err(|err| backend_error_response(&err))?;

    let mut registry = state.registry.lock().await;
    if let Some(record) = registry.get_mut(sandbox_id) {
        record.state = SandboxLifecycleState::Running;
        record.work_surface = work_surface;
    }
    tracing::info!(
        sandbox_id,
        root = %request.root,
        sandbox_type = request.sandbox_type.as_str(),
        "sandbox provisioned"
    );
    Ok(())
}

/// The workspace path as the **host** docker daemon must see it. When the
/// provider runs in a container, its `workspaces_dir` is a bind mount of
/// `workspaces_host_dir`; sibling-sandbox bind sources are the same relative
/// path under the host directory.
fn host_bind_source(
    workspace: &std::path::Path,
    workspaces_dir: &str,
    workspaces_host_dir: Option<&str>,
) -> Result<PathBuf, String> {
    let Some(host_dir) = workspaces_host_dir.map(str::trim).filter(|dir| !dir.is_empty()) else {
        return Ok(workspace.to_path_buf());
    };
    let relative = workspace.strip_prefix(workspaces_dir).map_err(|_| {
        format!(
            "workspace {} is not under SANDBOX_WORKSPACES_DIR {workspaces_dir}; \
             cannot map it to SANDBOX_WORKSPACES_HOST_DIR",
            workspace.display()
        )
    })?;
    Ok(PathBuf::from(host_dir).join(relative))
}

async fn mark_cleanup(state: &Arc<ProviderState>, sandbox_id: &str, cleanup: CleanupState) {
    let mut registry = state.registry.lock().await;
    if let Some(record) = registry.get_mut(sandbox_id) {
        record.cleanup = cleanup;
    }
}

#[derive(Deserialize)]
struct ListQuery {
    state: Option<String>,
}

async fn list_sandboxes(
    State(state): State<Arc<ProviderState>>,
    Query(query): Query<ListQuery>,
) -> Json<Vec<SandboxDescriptor>> {
    let registry = state.registry.lock().await;
    let strength = state.backend.strength_label();
    let list = registry
        .values()
        .filter(|record| {
            query.state.as_deref().is_none_or(|want| {
                serde_json::to_value(record.state)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s == want))
                    .unwrap_or(false)
            })
        })
        .map(|record| record.descriptor(strength))
        .collect();
    Json(list)
}

async fn get_sandbox(
    State(state): State<Arc<ProviderState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let registry = state.registry.lock().await;
    match registry.get(&id) {
        Some(record) => Json(record.descriptor(state.backend.strength_label())).into_response(),
        None => error_response(StatusCode::NOT_FOUND, "unknown_sandbox", format!("no sandbox {id}")),
    }
}

#[derive(Deserialize)]
struct LogsQuery {
    tail_bytes: Option<u64>,
}

async fn sandbox_logs(
    State(state): State<Arc<ProviderState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<LogsQuery>,
) -> Response {
    if !state.registry.lock().await.contains_key(&id) {
        return error_response(StatusCode::NOT_FOUND, "unknown_sandbox", format!("no sandbox {id}"));
    }
    let tail_bytes = query.tail_bytes.unwrap_or(DEFAULT_LOG_TAIL_BYTES);
    match state.backend.logs(&id, tail_bytes).await {
        Ok(logs) => {
            metrics::log_bytes_served(logs.content.len() as u64);
            let mut registry = state.registry.lock().await;
            if let Some(record) = registry.get_mut(&id) {
                record.usage.log_bytes = record.usage.log_bytes.max(logs.content.len() as u64);
            }
            Json(logs).into_response()
        }
        Err(err) => backend_error_response(&err),
    }
}

#[derive(Deserialize)]
struct DiffQuery {
    max_patch_bytes: Option<u64>,
}

async fn sandbox_diff(
    State(state): State<Arc<ProviderState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<DiffQuery>,
) -> Response {
    let workspace = {
        let registry = state.registry.lock().await;
        match registry.get(&id) {
            Some(record) => record.workspace.clone(),
            None => {
                return error_response(
                    StatusCode::NOT_FOUND,
                    "unknown_sandbox",
                    format!("no sandbox {id}"),
                )
            }
        }
    };
    let Some(workspace) = workspace else {
        return error_response(
            StatusCode::CONFLICT,
            "no_workspace",
            "sandbox has no workspace (adopted orphan or cleanup already ran)",
        );
    };
    let max_patch_bytes = query
        .max_patch_bytes
        .unwrap_or(DEFAULT_MAX_PATCH_BYTES)
        .min(state.config.max_log_bytes.max(DEFAULT_MAX_PATCH_BYTES));
    match compute_diff(&workspace, max_patch_bytes).await {
        Ok(diff) => Json(diff).into_response(),
        Err(detail) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "diff_failed", detail),
    }
}

/// Diff of the workspace against its provisioned state, computed host-side on
/// the bind-mounted directory. Untracked files are listed but their contents
/// are not embedded in the patch.
async fn compute_diff(workspace: &std::path::Path, max_patch_bytes: u64) -> Result<DiffResponse, String> {
    if !workspace.join(".git").exists() {
        return Ok(DiffResponse {
            changed_files: Vec::new(),
            patch: String::new(),
            patch_truncated: false,
        });
    }
    let status_args: Vec<String> = ["status", "--porcelain"].iter().map(|s| (*s).to_string()).collect();
    let mut spec = CommandSpec::new("git", &status_args);
    spec.cwd = Some(workspace);
    spec.timeout = Duration::from_secs(60);
    let status_out = run_command(spec).await.map_err(|e| e.to_string())?;
    if !status_out.success() {
        return Err(format!("git status failed: {}", status_out.stderr_lossy().trim()));
    }
    let changed_files = status_out
        .stdout_lossy()
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let (status, path) = line.split_at(2);
            Some(crate::protocol::ChangedFile {
                status: status.trim().to_string(),
                path: path.trim().to_string(),
            })
        })
        .collect();

    let diff_args: Vec<String> = ["diff", "HEAD"].iter().map(|s| (*s).to_string()).collect();
    let mut spec = CommandSpec::new("git", &diff_args);
    spec.cwd = Some(workspace);
    spec.timeout = Duration::from_secs(60);
    spec.max_output_bytes = usize::try_from(max_patch_bytes).unwrap_or(usize::MAX);
    let diff_out = run_command(spec).await.map_err(|e| e.to_string())?;
    if !diff_out.success() {
        return Err(format!("git diff failed: {}", diff_out.stderr_lossy().trim()));
    }
    Ok(DiffResponse {
        changed_files,
        patch: diff_out.stdout_lossy(),
        patch_truncated: diff_out.stdout_truncated,
    })
}

#[derive(Deserialize)]
struct DestroyQuery {
    #[serde(default)]
    preserve_workspace: bool,
}

async fn destroy_sandbox(
    State(state): State<Arc<ProviderState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<DestroyQuery>,
) -> Response {
    if !state.registry.lock().await.contains_key(&id) {
        return error_response(StatusCode::NOT_FOUND, "unknown_sandbox", format!("no sandbox {id}"));
    }
    teardown(&state, &id, query.preserve_workspace).await;
    metrics::sandbox_destroyed();
    let registry = state.registry.lock().await;
    let record = registry.get(&id).expect("record persists after teardown");
    Json(record.descriptor(state.backend.strength_label())).into_response()
}

/// Destroy the container and (optionally) the workspace; record the outcome.
/// Cleanup failures are visible, counted, and retried by the reaper.
async fn teardown(state: &Arc<ProviderState>, id: &str, preserve_workspace: bool) -> CleanupState {
    let container_result = state.backend.destroy(id).await;

    let workspace_result = if preserve_workspace {
        Ok(())
    } else {
        state.roots.remove_workspace(id).await
    };

    let cleanup = match (&container_result, &workspace_result) {
        (Ok(()), Ok(())) => CleanupState::Done,
        (Err(e), _) => CleanupState::Failed {
            reason: format!("container: {e}"),
        },
        (_, Err(e)) => CleanupState::Failed {
            reason: format!("workspace: {e}"),
        },
    };
    if matches!(cleanup, CleanupState::Failed { .. }) {
        metrics::cleanup_failure();
        tracing::warn!(sandbox_id = id, cleanup = ?cleanup, "sandbox cleanup failed");
    }

    let mut registry = state.registry.lock().await;
    if let Some(record) = registry.get_mut(id) {
        if record.finished_at.is_none() {
            record.finished_at = Some(OffsetDateTime::now_utc());
            record.usage.duration_ms = record.started.elapsed().as_millis() as u64;
        }
        record.state = SandboxLifecycleState::Destroyed;
        record.cleanup = cleanup.clone();
        record.preserve_workspace = preserve_workspace;
    }
    cleanup
}

/// Push the workspace's commits to the root's upstream. Host-side git with
/// the root's credentials; usable while the sandbox is Running or after it
/// Exited, as long as the workspace still exists.
async fn publish_sandbox(
    State(state): State<Arc<ProviderState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<PublishRequest>,
) -> Response {
    let (workspace, root_name, base_commit) = {
        let registry = state.registry.lock().await;
        match registry.get(&id) {
            None => {
                return error_response(
                    StatusCode::NOT_FOUND,
                    "unknown_sandbox",
                    format!("no sandbox {id}"),
                )
            }
            Some(record) => (
                record.workspace.clone(),
                record.root.clone(),
                record.work_surface.commit.clone(),
            ),
        }
    };
    let Some(workspace) = workspace else {
        return error_response(
            StatusCode::CONFLICT,
            "no_workspace",
            "sandbox has no workspace (adopted orphan or cleanup already ran)",
        );
    };
    let root = match state.roots.get(&root_name) {
        Ok(root) => root.clone(),
        Err(err) => return roots_error_response(&err),
    };
    match state
        .roots
        .publish_workspace(&root, &workspace, &request, base_commit.as_deref())
        .await
    {
        Ok(outcome) => {
            tracing::info!(
                sandbox_id = %id,
                branch = %outcome.branch,
                commits = outcome.commits_pushed,
                pushed = outcome.pushed,
                "sandbox workspace published"
            );
            Json(outcome).into_response()
        }
        // A push rejection is a conflict with upstream state, not a sync issue.
        Err(err @ RootsError::Git { .. }) => {
            error_response(StatusCode::CONFLICT, "publish_failed", err.to_string())
        }
        Err(err) => roots_error_response(&err),
    }
}

/// Selectable roots and images for dispatch UIs and model tools.
async fn catalog(State(state): State<Arc<ProviderState>>) -> Json<CatalogResponse> {
    let images = state
        .roots
        .images()
        .iter()
        .map(|image| CatalogImage {
            name: image.name.clone(),
            description: image.description.clone(),
            default: image.default,
        })
        .collect();
    let roots = state
        .roots
        .names()
        .into_iter()
        .filter_map(|name| state.roots.get(&name).ok().cloned())
        .map(|root| CatalogRoot {
            name: root.name.clone(),
            has_upstream: root.upstream.is_some(),
            default_ref: root.upstream.as_ref().map(|u| u.default_ref.clone()),
            default_image: root.default_image,
        })
        .collect();
    Json(CatalogResponse { images, roots })
}

async fn sync_root(
    State(state): State<Arc<ProviderState>>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let root = match state.roots.get(&name) {
        Ok(root) => root.clone(),
        Err(err) => return roots_error_response(&err),
    };
    match state.roots.sync_root(&root).await {
        Ok(head) => Json(SyncRootResponse {
            synced: true,
            head,
            error: None,
        })
        .into_response(),
        Err(err) => (
            StatusCode::CONFLICT,
            Json(SyncRootResponse {
                synced: false,
                head: None,
                error: Some(err.to_string()),
            }),
        )
            .into_response(),
    }
}

/// Background loop: notice exited containers, enforce deadlines, retry failed
/// cleanups, and adopt labeled orphans left by a previous provider process.
async fn reaper_loop(state: Arc<ProviderState>) {
    adopt_orphans(&state).await;
    loop {
        tokio::time::sleep(REAPER_INTERVAL).await;
        reap_once(&state).await;
    }
}

async fn adopt_orphans(state: &Arc<ProviderState>) {
    let adopted = match state.backend.list_adopted().await {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!(error = %e, "orphan adoption skipped (backend unavailable)");
            return;
        }
    };
    let mut registry = state.registry.lock().await;
    for orphan in adopted {
        if registry.contains_key(&orphan.id) {
            continue;
        }
        tracing::warn!(
            sandbox_id = %orphan.id,
            running = orphan.running,
            "adopting orphaned sandbox from a previous provider run"
        );
        let workspace = state.roots.workspace_dir(&orphan.id);
        let network = match orphan.labels.get("sandbox.network").map(String::as_str) {
            Some("restricted") => NetworkMode::Restricted,
            _ => NetworkMode::Open,
        };
        registry.insert(
            orphan.id.clone(),
            SandboxRecord {
                id: orphan.id.clone(),
                state: if orphan.running {
                    SandboxLifecycleState::Running
                } else {
                    SandboxLifecycleState::Exited
                },
                sandbox_type: SandboxType::Container,
                root: "(adopted)".to_string(),
                git_ref: None,
                work_surface: WorkSurface::default(),
                exit_code: None,
                usage: SandboxUsage::default(),
                cleanup: CleanupState::Pending,
                network,
                labels: orphan.labels,
                workspace: workspace.exists().then_some(workspace),
                created_at: OffsetDateTime::now_utc(),
                started: Instant::now(),
                // Adopted sandboxes get the default deadline from adoption
                // time; the true start time did not survive the restart.
                deadline: Some(
                    Instant::now() + Duration::from_secs(state.config.default_timeout_secs),
                ),
                finished_at: None,
                preserve_workspace: false,
            },
        );
    }
}

async fn reap_once(state: &Arc<ProviderState>) {
    let candidates: Vec<(String, Option<Instant>, SandboxLifecycleState, CleanupState)> = {
        let registry = state.registry.lock().await;
        registry
            .values()
            .map(|r| (r.id.clone(), r.deadline, r.state, r.cleanup.clone()))
            .collect()
    };

    for (id, deadline, lifecycle, cleanup) in candidates {
        match lifecycle {
            SandboxLifecycleState::Running => {
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    tracing::warn!(sandbox_id = %id, "sandbox exceeded its deadline; destroying");
                    metrics::sandbox_timed_out();
                    teardown(state, &id, false).await;
                    let mut registry = state.registry.lock().await;
                    if let Some(record) = registry.get_mut(&id) {
                        record.state = SandboxLifecycleState::Failed;
                        record.exit_code = None;
                    }
                    continue;
                }
                match state.backend.status(&id).await {
                    Ok(status) if !status.running => {
                        let disk_bytes = {
                            let registry = state.registry.lock().await;
                            registry.get(&id).and_then(|r| r.workspace.clone())
                        }
                        .map(|ws| dir_size_bytes(&ws));
                        let mut registry = state.registry.lock().await;
                        if let Some(record) = registry.get_mut(&id) {
                            record.state = SandboxLifecycleState::Exited;
                            record.exit_code = status.exit_code;
                            record.usage.oom_killed = status.oom_killed;
                            record.finished_at = Some(OffsetDateTime::now_utc());
                            record.usage.duration_ms = record.started.elapsed().as_millis() as u64;
                            record.usage.disk_bytes = disk_bytes;
                            tracing::info!(
                                sandbox_id = %id,
                                exit_code = ?status.exit_code,
                                duration_ms = record.usage.duration_ms,
                                "sandbox exited"
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(sandbox_id = %id, error = %e, "sandbox status poll failed");
                    }
                }
            }
            SandboxLifecycleState::Destroyed => {
                if matches!(cleanup, CleanupState::Failed { .. }) {
                    tracing::info!(sandbox_id = %id, "retrying failed sandbox cleanup");
                    teardown(state, &id, false).await;
                }
            }
            _ => {}
        }
    }
}

/// Recursive workspace size; runs in the reaper only, on exit, so the cost is
/// one walk per sandbox lifetime.
fn dir_size_bytes(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    total += dir_size_bytes(&entry.path());
                } else {
                    total += metadata.len();
                }
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::util::ServiceExt;

    fn test_config(token: &str) -> SandboxServerConfig {
        SandboxServerConfig {
            service_token: token.into(),
            roots_config_path: None,
            workspaces_host_dir: None,
            workspaces_dir: std::env::temp_dir()
                .join(format!("den-sbx-srv-test-{}", uuid::Uuid::new_v4().simple()))
                .to_string_lossy()
                .into_owned(),
            default_image: String::new(),
            max_concurrent: 2,
            default_timeout_secs: 900,
            max_log_bytes: 2 * 1024 * 1024,
        }
    }

    #[tokio::test]
    async fn health_is_open_and_reports_shape() {
        let app = create_sandbox_app(test_config("secret")).unwrap();
        let response = app
            .oneshot(Request::builder().uri("/sandbox/v1/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let health: HealthResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(health.ok);
        assert_eq!(health.active_sandboxes, 0);
    }

    #[tokio::test]
    async fn guarded_routes_require_token() {
        let app = create_sandbox_app(test_config("secret")).unwrap();
        let unauthorized = app
            .clone()
            .oneshot(Request::builder().uri("/sandbox/v1/sandboxes").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/sandbox/v1/sandboxes")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn create_rejects_unknown_root_and_unimplemented_type() {
        let app = create_sandbox_app(test_config("")).unwrap();
        let request_body = serde_json::json!({
            "root": "nope",
            "sandbox_type": "container",
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sandbox/v1/sandboxes")
                    .header("content-type", "application/json")
                    .body(Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let error: ErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error.kind, "unknown_root");
    }

    #[tokio::test]
    async fn unknown_sandbox_is_404() {
        let app = create_sandbox_app(test_config("")).unwrap();
        for uri in [
            "/sandbox/v1/sandboxes/nope",
            "/sandbox/v1/sandboxes/nope/logs",
            "/sandbox/v1/sandboxes/nope/diff",
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        }
        let publish = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sandbox/v1/sandboxes/nope/publish")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"branch": "den/job-x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(publish.status(), StatusCode::NOT_FOUND);
    }

    /// Config whose roots file declares an image catalog and one plain root.
    fn test_config_with_catalog() -> SandboxServerConfig {
        let mut config = test_config("");
        let dir = PathBuf::from(&config.workspaces_dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("src-root");
        std::fs::create_dir_all(&source).unwrap();
        let roots_path = dir.join("roots.json");
        std::fs::write(
            &roots_path,
            serde_json::json!({
                "images": [
                    {"name": "base", "image": "bears/sandbox:latest", "default": true,
                     "description": "armature base"},
                    {"name": "rust", "image": "bears/sandbox-rust:latest"}
                ],
                "roots": [
                    {"name": "scratch", "path": source.to_string_lossy(), "default_image": "rust"}
                ]
            })
            .to_string(),
        )
        .unwrap();
        config.roots_config_path = Some(roots_path.to_string_lossy().into_owned());
        config
    }

    #[test]
    fn bind_source_maps_container_path_to_host_path() {
        let workspace = std::path::Path::new("/var/lib/bears/sandbox-workspaces/workspaces/s1");
        // Provider on the host: unchanged.
        assert_eq!(
            host_bind_source(workspace, "/var/lib/bears/sandbox-workspaces", None).unwrap(),
            workspace
        );
        // Containerized provider: same relative path under the host dir.
        assert_eq!(
            host_bind_source(
                workspace,
                "/var/lib/bears/sandbox-workspaces",
                Some("/srv/coolify/bears-ws"),
            )
            .unwrap(),
            std::path::Path::new("/srv/coolify/bears-ws/workspaces/s1")
        );
        // A workspace outside the configured dir cannot be mapped.
        assert!(host_bind_source(workspace, "/somewhere/else", Some("/host")).is_err());
    }

    #[tokio::test]
    async fn catalog_lists_images_and_roots() {
        let app = create_sandbox_app(test_config_with_catalog()).unwrap();
        let response = app
            .oneshot(Request::builder().uri("/sandbox/v1/catalog").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let catalog: CatalogResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(catalog.images.len(), 2);
        assert!(catalog.images.iter().any(|image| image.name == "base" && image.default));
        assert_eq!(catalog.roots.len(), 1);
        assert_eq!(catalog.roots[0].default_image.as_deref(), Some("rust"));
        assert!(!catalog.roots[0].has_upstream);
    }

    #[tokio::test]
    async fn create_rejects_unknown_catalog_image() {
        let app = create_sandbox_app(test_config_with_catalog()).unwrap();
        let request_body = serde_json::json!({
            "root": "scratch",
            "sandbox_type": "container",
            "image": "not-in-catalog",
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sandbox/v1/sandboxes")
                    .header("content-type", "application/json")
                    .body(Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Image names resolve before any backend/probe work, so this is a
        // deterministic rejection even on hosts without a docker daemon.
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let error: ErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error.kind, "unknown_image");
        assert!(error.error.contains("base"), "lists catalog: {}", error.error);
    }
}
