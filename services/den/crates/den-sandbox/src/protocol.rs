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
    /// Container image override; falls back to the provider's default image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
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
