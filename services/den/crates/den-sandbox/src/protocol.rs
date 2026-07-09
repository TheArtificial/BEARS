//! Wire types shared by the provider API (`server`) and the Den-side client.
//!
//! Everything here is plain serde data — no behavior — mirroring the role
//! `bearwire-protocol` plays for the armature edge.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Execution boundary type. Only [`SandboxType::Container`] is implemented;
/// the rest exist so requests name their intent explicitly and get an explicit
/// "unimplemented" error instead of a silently different boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxType {
    Container,
    LocalWorkspaceReadonly,
    LocalWorkspaceWritable,
    EphemeralCopy,
    RemoteEphemeral,
}

impl SandboxType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::LocalWorkspaceReadonly => "local_workspace_readonly",
            Self::LocalWorkspaceWritable => "local_workspace_writable",
            Self::EphemeralCopy => "ephemeral_copy",
            Self::RemoteEphemeral => "remote_ephemeral",
        }
    }
}

/// Network posture of a sandbox. `Restricted` (the default) attaches the
/// container to an internal network whose only way out is a TCP relay to the
/// Den callback endpoint — task code cannot reach anything else. `Open` is
/// the pre-v1 behavior: default bridge, unrestricted egress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    #[default]
    Restricted,
    Open,
}

impl NetworkMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Restricted => "restricted",
            Self::Open => "open",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxLifecycleState {
    Provisioning,
    Running,
    Exited,
    Failed,
    Destroyed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum CleanupState {
    Pending,
    Done,
    Failed { reason: String },
}

/// Resource bounds for one sandbox. `timeout_secs` is always enforced (the
/// provider's reaper destroys overdue sandboxes); the rest apply only where the
/// backend supports them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SandboxLimits {
    pub timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pids: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_log_bytes: Option<u64>,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            timeout_secs: 900,
            memory_mb: None,
            cpus: None,
            pids: None,
            max_log_bytes: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateSandboxRequest {
    /// Name of a configured root on the sandbox host.
    pub root: String,
    /// Git ref to provision at (git-backed roots only). Defaults to the
    /// root's upstream default ref, or the pristine clone's HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    pub sandbox_type: SandboxType,
    /// Whether the work needs a writable workspace.
    #[serde(default = "default_true")]
    pub requires_write: bool,
    /// Catalog image **name** (not a raw reference) — resolved against the
    /// provider's image catalog. Falls back to the root's default image, the
    /// catalog default, then the provider's `SANDBOX_IMAGE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Network posture; defaults to [`NetworkMode::Restricted`].
    #[serde(default)]
    pub network: NetworkMode,
    /// Environment injected into the sandbox (armature credentials, work
    /// order id, ...). Values are never logged by the provider.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub limits: SandboxLimits,
    /// Opaque caller labels (e.g. `work_run_id`), stamped onto the container
    /// so orphans can be reconciled after a provider restart.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

fn default_true() -> bool {
    true
}

/// Facts recognized about the workspace at provision time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSurface {
    pub is_git: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub dirty: bool,
    pub untracked_present: bool,
    pub writable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub language_hints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_manager_hints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lockfiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_command_hints: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxUsage {
    pub duration_ms: u64,
    /// Approximate log bytes observed (from the most recent full log read).
    pub log_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oom_killed: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandboxDescriptor {
    pub id: String,
    pub state: SandboxLifecycleState,
    pub sandbox_type: SandboxType,
    /// Honest human-readable statement of the isolation actually provided.
    pub strength_label: String,
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    pub work_surface: WorkSurface,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    pub usage: SandboxUsage,
    pub cleanup: CleanupState,
    /// Network posture the sandbox actually runs with.
    #[serde(default)]
    pub network: NetworkMode,
    pub labels: BTreeMap<String, String>,
    /// RFC 3339 timestamps.
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogsResponse {
    /// UTF-8 (lossy) tail of the sandbox's combined stdout/stderr.
    pub content: String,
    pub truncated: bool,
    pub tail_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    /// Porcelain status code, e.g. `M`, `A`, `D`, `??`.
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffResponse {
    pub changed_files: Vec<ChangedFile>,
    /// Unified diff of tracked changes vs the provisioned ref. Contents of
    /// untracked files are not included (they are listed in `changed_files`).
    pub patch: String,
    pub patch_truncated: bool,
}

/// Publish (push) a sandbox workspace's commits to the root's upstream.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishRequest {
    /// Target branch on the upstream (`refs/heads/<branch>`).
    pub branch: String,
    /// Commit any uncommitted workspace changes before pushing so nothing is
    /// silently dropped.
    #[serde(default = "default_true")]
    pub auto_commit_leftovers: bool,
    /// The root's default ref is refused as a target unless this is set —
    /// pushing to `main` must always be an explicit choice.
    #[serde(default)]
    pub allow_default_ref: bool,
    /// Caller label used in the auto-commit message (e.g. the work run id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishResponse {
    pub branch: String,
    /// Workspace HEAD after any auto-commit.
    pub commit: String,
    /// Commits ahead of the provisioned base that the push carried.
    pub commits_pushed: u64,
    /// Whether leftover changes were auto-committed before the push.
    pub auto_committed: bool,
    /// False when there was nothing beyond the provisioned base to push (the
    /// push is skipped rather than creating an empty branch move).
    pub pushed: bool,
}

/// One selectable image in the provider's catalog (from the roots file).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogImage {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub default: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogRoot {
    pub name: String,
    pub has_upstream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_ref: Option<String>,
    /// Catalog image name this root defaults to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_image: Option<String>,
}

/// Selectable roots and images, for dispatch UIs and model tools. Image
/// references themselves stay on the sandbox host; only names travel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogResponse {
    pub images: Vec<CatalogImage>,
    pub roots: Vec<CatalogRoot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootStatus {
    pub name: String,
    pub has_upstream: bool,
    /// Whether the pristine clone / source path currently looks usable.
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncRootResponse {
    pub synced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub backend_available: bool,
    pub active_sandboxes: usize,
    pub roots: Vec<RootStatus>,
}

/// Uniform error body for non-2xx provider responses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Stable machine-readable kind, e.g. `unimplemented_type`, `queue_full`.
    pub kind: String,
    pub error: String,
}
