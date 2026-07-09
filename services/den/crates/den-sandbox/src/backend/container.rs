//! Container backend via the `docker` CLI.
//!
//! ponytail: this shells out to the docker CLI rather than speaking the
//! engine API (bollard). The operations needed are exactly run / inspect /
//! logs / rm / ps, the CLI output is trivially inspectable, and the same
//! binary path works against podman (`DOCKER_BIN=podman`). Upgrade path:
//! switch to bollard if we ever need event streams or tighter lifecycle hooks.
//!
//! ponytail: containers run on the default bridge network with unrestricted
//! egress — the armature must reach Den, and v1 runs trusted tasks only.
//! Upgrade path: per-sandbox network plus a proxy allowlist before running
//! untrusted code.

use super::{AdoptedSandbox, BackendError, BackendStatus, ProvisionSpec};
use crate::proc::{run_command, CaptureWindow, CommandSpec};
use crate::protocol::LogsResponse;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

const DOCKER_TIMEOUT: Duration = Duration::from_secs(120);
const DOCKER_OUTPUT_CAP: usize = 256 * 1024;
const CONTAINER_PREFIX: &str = "den-sbx-";
pub const SANDBOX_LABEL: &str = "den.sandbox";

pub struct DockerCliBackend {
    docker_bin: String,
    /// Directory for transient `--env-file` files (mode 0600, removed after start).
    env_file_dir: PathBuf,
    max_log_bytes: u64,
}

impl DockerCliBackend {
    pub fn new(env_file_dir: PathBuf, max_log_bytes: u64) -> Self {
        Self {
            docker_bin: std::env::var("DOCKER_BIN").unwrap_or_else(|_| "docker".to_string()),
            env_file_dir,
            max_log_bytes,
        }
    }

    pub fn strength_label(&self) -> &'static str {
        "container: OS namespace isolation via the local container runtime; \
         unrestricted network egress; workspace bind-mounted read-write"
    }

    pub async fn probe(&self) -> bool {
        self.docker(&["info", "--format", "{{.ServerVersion}}"], None)
            .await
            .map(|out| out.success())
            .unwrap_or(false)
    }

    pub async fn provision(&self, spec: &ProvisionSpec) -> Result<(), BackendError> {
        let container = container_name(&spec.id);
        let env_file = self.write_env_file(&spec.id, &spec.env)?;

        let mut args: Vec<String> = vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            container.clone(),
            "--label".into(),
            format!("{SANDBOX_LABEL}=1"),
            "--env-file".into(),
            env_file.to_string_lossy().into_owned(),
            "-v".into(),
            format!("{}:/workspace", spec.workspace.to_string_lossy()),
            "-w".into(),
            "/workspace".into(),
        ];
        for (key, value) in &spec.labels {
            args.push("--label".into());
            args.push(format!("den.{key}={value}"));
        }
        if let Some(memory_mb) = spec.memory_mb {
            args.push("--memory".into());
            args.push(format!("{memory_mb}m"));
        }
        if let Some(cpus) = spec.cpus {
            args.push("--cpus".into());
            args.push(format!("{cpus}"));
        }
        if let Some(pids) = spec.pids {
            args.push("--pids-limit".into());
            args.push(format!("{pids}"));
        }
        args.push(spec.image.clone());

        let result = self.docker_owned(&args, None).await;
        // The env file exists only for the window between write and start;
        // remove it regardless of outcome. (ponytail: the env is still visible
        // via `docker inspect` on this host — acceptable while the sandbox
        // host is operator-controlled; upgrade path is a secrets mount.)
        let _ = std::fs::remove_file(&env_file);
        let out = result?;
        if out.success() {
            Ok(())
        } else {
            Err(BackendError::Operation {
                id: spec.id.clone(),
                detail: format!("docker run failed: {}", out.stderr_lossy().trim()),
            })
        }
    }

    pub async fn status(&self, id: &str) -> Result<BackendStatus, BackendError> {
        let container = container_name(id);
        let out = self
            .docker(
                &[
                    "inspect",
                    "--format",
                    "{{.State.Running}} {{.State.ExitCode}} {{.State.OOMKilled}}",
                    &container,
                ],
                None,
            )
            .await?;
        if !out.success() {
            return Err(BackendError::Operation {
                id: id.to_string(),
                detail: format!("docker inspect failed: {}", out.stderr_lossy().trim()),
            });
        }
        let stdout = out.stdout_lossy();
        let mut parts = stdout.split_whitespace();
        let running = parts.next() == Some("true");
        let exit_code = parts.next().and_then(|s| s.parse::<i64>().ok());
        let oom_killed = parts.next().map(|s| s == "true");
        Ok(BackendStatus {
            running,
            // Docker reports ExitCode 0 while still running; only meaningful once stopped.
            exit_code: if running { None } else { exit_code },
            oom_killed,
        })
    }

    pub async fn logs(&self, id: &str, tail_bytes: u64) -> Result<LogsResponse, BackendError> {
        let container = container_name(id);
        let tail_bytes = tail_bytes.min(self.max_log_bytes).max(1);
        let args: Vec<String> = vec!["logs".into(), container];
        let mut spec = CommandSpec::new(&self.docker_bin, &args);
        spec.timeout = DOCKER_TIMEOUT;
        spec.max_output_bytes = usize::try_from(tail_bytes).unwrap_or(usize::MAX);
        spec.window = CaptureWindow::Tail;
        let out = run_command(spec).await?;
        if out.exit_code != Some(0) && !out.timed_out {
            return Err(BackendError::Operation {
                id: id.to_string(),
                detail: format!("docker logs failed: {}", out.stderr_lossy().trim()),
            });
        }
        // docker logs multiplexes the container's stdout/stderr onto both pipes.
        let mut content = out.stdout_lossy();
        let stderr = out.stderr_lossy();
        if !stderr.is_empty() {
            content.push_str(&stderr);
        }
        Ok(LogsResponse {
            content,
            truncated: out.stdout_truncated || out.stderr_truncated,
            tail_bytes,
        })
    }

    pub async fn destroy(&self, id: &str) -> Result<(), BackendError> {
        let container = container_name(id);
        let out = self.docker(&["rm", "-f", &container], None).await?;
        if out.success() {
            return Ok(());
        }
        let stderr = out.stderr_lossy();
        // Already gone counts as destroyed.
        if stderr.contains("No such container") {
            return Ok(());
        }
        Err(BackendError::Operation {
            id: id.to_string(),
            detail: format!("docker rm failed: {}", stderr.trim()),
        })
    }

    pub async fn list_adopted(&self) -> Result<Vec<AdoptedSandbox>, BackendError> {
        let out = self
            .docker(
                &[
                    "ps",
                    "-a",
                    "--filter",
                    &format!("label={SANDBOX_LABEL}=1"),
                    "--format",
                    "{{.Names}}\t{{.State}}\t{{.Labels}}",
                ],
                None,
            )
            .await?;
        if !out.success() {
            return Err(BackendError::RuntimeUnavailable(
                out.stderr_lossy().trim().to_string(),
            ));
        }
        let mut adopted = Vec::new();
        for line in out.stdout_lossy().lines() {
            let mut fields = line.split('\t');
            let (Some(name), Some(state)) = (fields.next(), fields.next()) else {
                continue;
            };
            let Some(id) = name.strip_prefix(CONTAINER_PREFIX) else {
                continue;
            };
            let labels: BTreeMap<String, String> = fields
                .next()
                .unwrap_or_default()
                .split(',')
                .filter_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    let key = key.strip_prefix("den.")?;
                    if key == "sandbox" {
                        None
                    } else {
                        Some((key.to_string(), value.to_string()))
                    }
                })
                .collect();
            adopted.push(AdoptedSandbox {
                id: id.to_string(),
                labels,
                running: state == "running",
            });
        }
        Ok(adopted)
    }

    fn write_env_file(
        &self,
        id: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<PathBuf, BackendError> {
        std::fs::create_dir_all(&self.env_file_dir).map_err(|e| BackendError::Operation {
            id: id.to_string(),
            detail: format!("create env-file dir: {e}"),
        })?;
        let path = self.env_file_dir.join(format!("{id}.env"));
        let mut body = String::new();
        for (key, value) in env {
            // --env-file format is KEY=VALUE per line; values with newlines
            // are unsupported and rejected rather than mangled.
            if key.contains('\n') || value.contains('\n') {
                return Err(BackendError::Operation {
                    id: id.to_string(),
                    detail: format!("env var {key} contains a newline"),
                });
            }
            body.push_str(key);
            body.push('=');
            body.push_str(value);
            body.push('\n');
        }
        std::fs::write(&path, body).map_err(|e| BackendError::Operation {
            id: id.to_string(),
            detail: format!("write env file: {e}"),
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(path)
    }

    async fn docker(
        &self,
        args: &[&str],
        cwd: Option<&std::path::Path>,
    ) -> Result<crate::proc::CommandOutput, BackendError> {
        let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        self.docker_owned(&args, cwd).await
    }

    async fn docker_owned(
        &self,
        args: &[String],
        cwd: Option<&std::path::Path>,
    ) -> Result<crate::proc::CommandOutput, BackendError> {
        let mut spec = CommandSpec::new(&self.docker_bin, args);
        spec.cwd = cwd;
        spec.timeout = DOCKER_TIMEOUT;
        spec.max_output_bytes = DOCKER_OUTPUT_CAP;
        Ok(run_command(spec).await?)
    }
}

pub fn container_name(id: &str) -> String {
    format!("{CONTAINER_PREFIX}{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_names_round_trip() {
        let name = container_name("abc123");
        assert_eq!(name, "den-sbx-abc123");
        assert_eq!(name.strip_prefix(CONTAINER_PREFIX), Some("abc123"));
    }

    #[test]
    fn env_file_rejects_newlines() {
        let backend = DockerCliBackend::new(std::env::temp_dir().join("den-sbx-env-test"), 1024);
        let mut env = BTreeMap::new();
        env.insert("GOOD".to_string(), "value".to_string());
        env.insert("BAD".to_string(), "line1\nline2".to_string());
        assert!(backend.write_env_file("t1", &env).is_err());
    }
}
