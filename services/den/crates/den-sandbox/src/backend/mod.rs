//! Sandbox execution backends. Only the container backend is implemented;
//! requests for other sandbox types are rejected explicitly by the policy
//! layer rather than silently degraded (never pretend a sandbox is stronger
//! than it is).

pub mod container;

use crate::protocol::LogsResponse;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("container runtime unavailable: {0}")]
    RuntimeUnavailable(String),
    #[error("sandbox {id}: {detail}")]
    Operation { id: String, detail: String },
    #[error("{what} failed: {detail}")]
    CommandFailed { what: String, detail: String },
    #[error(transparent)]
    Proc(#[from] crate::proc::ProcError),
}

pub struct ProvisionSpec {
    pub id: String,
    /// Workspace path as seen by the provider process (local filesystem
    /// operations: git, diff, cleanup).
    pub workspace: PathBuf,
    /// Workspace path as the **host** docker daemon sees it — the bind-mount
    /// source for the sandbox container. Differs from `workspace` only when
    /// the provider itself runs in a container.
    pub workspace_bind_source: PathBuf,
    pub image: String,
    pub env: BTreeMap<String, String>,
    pub network: crate::protocol::NetworkMode,
    pub memory_mb: Option<u64>,
    pub cpus: Option<f64>,
    pub pids: Option<u64>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct BackendStatus {
    pub running: bool,
    pub exit_code: Option<i64>,
    pub oom_killed: Option<bool>,
}

/// A sandbox discovered on the backend (by label) that this provider process
/// does not have in its in-memory registry — i.e. left over from a previous
/// provider run. Labels are the durable source of truth on the sandbox host.
#[derive(Clone, Debug)]
pub struct AdoptedSandbox {
    pub id: String,
    pub labels: BTreeMap<String, String>,
    pub running: bool,
}

/// Closed set of backends (an enum, not a trait object: the methods are async
/// and the set is small and known).
pub enum Backend {
    DockerCli(container::DockerCliBackend),
}

impl Backend {
    pub async fn probe(&self) -> bool {
        match self {
            Self::DockerCli(b) => b.probe().await,
        }
    }

    pub fn strength_label(&self) -> &'static str {
        match self {
            Self::DockerCli(b) => b.strength_label(),
        }
    }

    pub async fn provision(&self, spec: &ProvisionSpec) -> Result<(), BackendError> {
        match self {
            Self::DockerCli(b) => b.provision(spec).await,
        }
    }

    pub async fn status(&self, id: &str) -> Result<BackendStatus, BackendError> {
        match self {
            Self::DockerCli(b) => b.status(id).await,
        }
    }

    pub async fn logs(&self, id: &str, tail_bytes: u64) -> Result<LogsResponse, BackendError> {
        match self {
            Self::DockerCli(b) => b.logs(id, tail_bytes).await,
        }
    }

    pub async fn destroy(&self, id: &str) -> Result<(), BackendError> {
        match self {
            Self::DockerCli(b) => b.destroy(id).await,
        }
    }

    pub async fn list_adopted(&self) -> Result<Vec<AdoptedSandbox>, BackendError> {
        match self {
            Self::DockerCli(b) => b.list_adopted().await,
        }
    }

    /// Engine image store as `docker image ls --format {{json .}}` lines.
    pub async fn image_ls_json(&self) -> Result<String, BackendError> {
        match self {
            Self::DockerCli(b) => b.image_ls_json().await,
        }
    }

    /// Human-readable `docker system df` summary, when the engine answers.
    pub async fn system_df(&self) -> Option<String> {
        match self {
            Self::DockerCli(b) => b.system_df().await,
        }
    }

    /// Remove an image from the engine store. `Ok(Err(stderr))` = docker
    /// refused (e.g. image in use); `Err` = the engine was unreachable.
    pub async fn remove_image(&self, reference: &str) -> Result<Result<(), String>, BackendError> {
        match self {
            Self::DockerCli(b) => b.remove_image(reference).await,
        }
    }

    /// The docker binary this backend invokes (for spawning long-running
    /// image operations with the same engine connection env).
    pub fn docker_bin(&self) -> &str {
        match self {
            Self::DockerCli(b) => b.docker_bin(),
        }
    }
}
