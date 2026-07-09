//! Syncable workspace roots.
//!
//! A root is either a plain directory on the sandbox host (copied per
//! sandbox) or a git upstream the provider mirrors as a **pristine,
//! server-managed bare clone** — never a human-edited working tree. Sync is
//! fetch/fast-forward only; a non-fast-forward upstream is reported, never
//! forced. Per-root credentials live on the sandbox host (env var name or ssh
//! key path in the roots file), so no repo credentials transit Den or jobs.

use crate::proc::{run_command, CommandSpec};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const GIT_TIMEOUT: Duration = Duration::from_secs(300);
const GIT_OUTPUT_CAP: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RootsError {
    #[error("roots config {path}: {source}")]
    ConfigRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("roots config {path}: {source}")]
    ConfigParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("unknown root '{0}'")]
    UnknownRoot(String),
    #[error("root '{name}': {detail}")]
    Git { name: String, detail: String },
    #[error("root '{name}': {detail}")]
    Workspace { name: String, detail: String },
    #[error(transparent)]
    Proc(#[from] crate::proc::ProcError),
}

#[derive(Clone, Debug, Deserialize)]
pub struct RootsFile {
    pub roots: Vec<SyncableRoot>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SyncableRoot {
    pub name: String,
    /// For upstream-less roots: source directory on this host, copied per
    /// sandbox. Ignored when `upstream` is set (the pristine clone is the source).
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub upstream: Option<GitUpstream>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitUpstream {
    pub url: String,
    #[serde(default = "default_ref")]
    pub default_ref: String,
    #[serde(default)]
    pub credential: Option<RootCredential>,
}

fn default_ref() -> String {
    "main".to_string()
}

/// Where a root's upstream credential comes from. Declared per-root in the
/// roots file; the secret itself never appears in the file, in logs, or on
/// command lines.
/// JSON shape: `{"token_env": "SITE_GIT_TOKEN"}` or
/// `{"ssh_key_path": "/etc/bears/keys/den"}`.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum RootCredential {
    /// Name of an env var (on the sandbox host) holding an HTTPS token.
    TokenEnv { token_env: String },
    /// Path to an SSH private key on the sandbox host.
    SshKeyPath { ssh_key_path: String },
}

pub struct RootsManager {
    roots: BTreeMap<String, SyncableRoot>,
    workspaces_dir: PathBuf,
}

impl RootsManager {
    pub fn load(config_path: Option<&str>, workspaces_dir: &str) -> Result<Self, RootsError> {
        let roots = match config_path {
            None => BTreeMap::new(),
            Some(path) => {
                let raw = std::fs::read_to_string(path).map_err(|source| {
                    RootsError::ConfigRead {
                        path: path.to_string(),
                        source,
                    }
                })?;
                let file: RootsFile =
                    serde_json::from_str(&raw).map_err(|source| RootsError::ConfigParse {
                        path: path.to_string(),
                        source,
                    })?;
                file.roots
                    .into_iter()
                    .map(|root| (root.name.clone(), root))
                    .collect()
            }
        };
        Ok(Self {
            roots,
            workspaces_dir: PathBuf::from(workspaces_dir),
        })
    }

    pub fn names(&self) -> Vec<String> {
        self.roots.keys().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Result<&SyncableRoot, RootsError> {
        self.roots
            .get(name)
            .ok_or_else(|| RootsError::UnknownRoot(name.to_string()))
    }

    pub fn workspace_dir(&self, sandbox_id: &str) -> PathBuf {
        self.workspaces_dir.join("workspaces").join(sandbox_id)
    }

    fn pristine_dir(&self, root: &SyncableRoot) -> PathBuf {
        self.workspaces_dir
            .join("pristine")
            .join(format!("{}.git", root.name))
    }

    /// Cheap liveness check for `GET /health`.
    pub fn status(&self, root: &SyncableRoot) -> (bool, Option<String>) {
        if root.upstream.is_some() {
            let pristine = self.pristine_dir(root);
            if pristine.is_dir() {
                (true, None)
            } else {
                (false, Some("pristine clone not created yet".to_string()))
            }
        } else {
            match &root.path {
                Some(path) if Path::new(path).is_dir() => (true, None),
                Some(path) => (false, Some(format!("source path {path} missing"))),
                None => (false, Some("root has neither path nor upstream".to_string())),
            }
        }
    }

    /// Ensure the pristine bare clone exists and fast-forward it from
    /// upstream. Bare clones have no working tree, so there is nothing to
    /// dirty or reset; a non-fast-forward fetch fails and is reported as-is.
    /// Returns the resolved head of the root's default ref. No-op for
    /// upstream-less roots.
    pub async fn sync_root(&self, root: &SyncableRoot) -> Result<Option<String>, RootsError> {
        let Some(upstream) = &root.upstream else {
            return Ok(None);
        };
        let pristine = self.pristine_dir(root);
        let env = credential_env(root, upstream)?;

        if !pristine.is_dir() {
            if let Some(parent) = pristine.parent() {
                std::fs::create_dir_all(parent).map_err(|e| RootsError::Workspace {
                    name: root.name.clone(),
                    detail: format!("create pristine dir: {e}"),
                })?;
            }
            self.git(
                root,
                None,
                &env,
                &[
                    "clone",
                    "--bare",
                    &upstream.url,
                    &pristine.to_string_lossy(),
                ],
            )
            .await?;
        } else {
            // Non-forcing refspec: refuses non-fast-forward updates instead of
            // rewriting history under a provisioned ref.
            self.git(
                root,
                Some(&pristine),
                &env,
                &["fetch", "origin", "refs/heads/*:refs/heads/*", "--prune"],
            )
            .await?;
        }

        let head = self
            .git(
                root,
                Some(&pristine),
                &env,
                &["rev-parse", &upstream.default_ref],
            )
            .await?;
        Ok(Some(head.trim().to_string()))
    }

    /// Materialize a workspace for one sandbox: a self-contained local clone
    /// of the pristine repo at the requested ref, or a copy of the plain
    /// source directory. The workspace is fully independent of the source
    /// (safe to bind-mount into a container and to delete afterwards).
    pub async fn provision_workspace(
        &self,
        root: &SyncableRoot,
        git_ref: Option<&str>,
        sandbox_id: &str,
    ) -> Result<PathBuf, RootsError> {
        let workspace = self.workspace_dir(sandbox_id);
        if let Some(parent) = workspace.parent() {
            std::fs::create_dir_all(parent).map_err(|e| RootsError::Workspace {
                name: root.name.clone(),
                detail: format!("create workspaces dir: {e}"),
            })?;
        }

        if let Some(upstream) = &root.upstream {
            let pristine = self.pristine_dir(root);
            if !pristine.is_dir() {
                self.sync_root(root).await?;
            }
            // A full local clone (not `git worktree`): worktrees reference the
            // pristine repo by absolute host path, which breaks git inside a
            // container that only sees the bind-mounted workspace.
            self.git(
                root,
                None,
                &[],
                &[
                    "clone",
                    &pristine.to_string_lossy(),
                    &workspace.to_string_lossy(),
                ],
            )
            .await?;
            let reference = git_ref.unwrap_or(&upstream.default_ref);
            self.git(
                root,
                Some(&workspace),
                &[],
                &["checkout", "--detach", reference],
            )
            .await
            .inspect_err(|_| {
                let _ = std::fs::remove_dir_all(&workspace);
            })?;
        } else {
            let source = root.path.as_deref().ok_or_else(|| RootsError::Workspace {
                name: root.name.clone(),
                detail: "root has neither path nor upstream".to_string(),
            })?;
            copy_dir_recursive(Path::new(source), &workspace).map_err(|e| {
                let _ = std::fs::remove_dir_all(&workspace);
                RootsError::Workspace {
                    name: root.name.clone(),
                    detail: format!("copy {source}: {e}"),
                }
            })?;
        }
        Ok(workspace)
    }

    pub async fn remove_workspace(&self, sandbox_id: &str) -> Result<(), std::io::Error> {
        let workspace = self.workspace_dir(sandbox_id);
        if workspace.exists() {
            tokio::fs::remove_dir_all(&workspace).await?;
        }
        Ok(())
    }

    async fn git(
        &self,
        root: &SyncableRoot,
        cwd: Option<&Path>,
        env: &[(String, String)],
        args: &[&str],
    ) -> Result<String, RootsError> {
        let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut spec = CommandSpec::new("git", &args);
        spec.cwd = cwd;
        spec.env = env;
        spec.timeout = GIT_TIMEOUT;
        spec.max_output_bytes = GIT_OUTPUT_CAP;
        let out = run_command(spec).await?;
        if out.success() {
            Ok(out.stdout_lossy())
        } else {
            Err(RootsError::Git {
                name: root.name.clone(),
                detail: format!(
                    "git {} failed (exit {:?}{}): {}",
                    args.first().map_or("", String::as_str),
                    out.exit_code,
                    if out.timed_out { ", timed out" } else { "" },
                    out.stderr_lossy().trim(),
                ),
            })
        }
    }
}

/// Build the env for credentialed git operations. Tokens ride in
/// `GIT_ASKPASS`-style env (via a helper value), keys via `GIT_SSH_COMMAND` —
/// never on the command line, never logged.
fn credential_env(
    root: &SyncableRoot,
    upstream: &GitUpstream,
) -> Result<Vec<(String, String)>, RootsError> {
    let mut env = vec![
        // Fail fast instead of prompting when credentials are missing.
        ("GIT_TERMINAL_PROMPT".to_string(), "0".to_string()),
    ];
    match &upstream.credential {
        None => {}
        Some(RootCredential::TokenEnv { token_env }) => {
            let token = std::env::var(token_env).map_err(|_| RootsError::Git {
                name: root.name.clone(),
                detail: format!("credential env var {token_env} is not set on this host"),
            })?;
            // `echo` as ASKPASS: git calls it for username and password; the
            // token works as both for the common providers (x-access-token).
            env.push(("GIT_ASKPASS".to_string(), "echo".to_string()));
            env.push(("GIT_CONFIG_COUNT".to_string(), "1".to_string()));
            env.push((
                "GIT_CONFIG_KEY_0".to_string(),
                "credential.helper".to_string(),
            ));
            env.push((
                "GIT_CONFIG_VALUE_0".to_string(),
                format!("!f() {{ echo username=x-access-token; echo password={token}; }}; f"),
            ));
        }
        Some(RootCredential::SshKeyPath { ssh_key_path }) => {
            env.push((
                "GIT_SSH_COMMAND".to_string(),
                format!("ssh -i {ssh_key_path} -o IdentitiesOnly=yes -o BatchMode=yes"),
            ));
        }
    }
    Ok(env)
}

fn copy_dir_recursive(source: &Path, dest: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dest.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if file_type.is_symlink() {
            // Preserve in-tree symlinks; a dangling copy is better than
            // silently materializing content from outside the root.
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_roots_file() {
        let raw = r#"{
            "roots": [
                {"name": "scratch", "path": "/srv/scratch"},
                {"name": "den", "upstream": {"url": "git@example.com:den.git",
                    "default_ref": "main",
                    "credential": {"ssh_key_path": "/etc/bears/keys/den"}}},
                {"name": "site", "upstream": {"url": "https://example.com/site.git",
                    "credential": {"token_env": "SITE_GIT_TOKEN"}}}
            ]
        }"#;
        let file: RootsFile = serde_json::from_str(raw).unwrap();
        assert_eq!(file.roots.len(), 3);
        assert!(file.roots[0].upstream.is_none());
        let den = &file.roots[1];
        assert!(matches!(
            den.upstream.as_ref().unwrap().credential,
            Some(RootCredential::SshKeyPath { .. })
        ));
        // default_ref defaults to "main" when omitted.
        assert_eq!(file.roots[2].upstream.as_ref().unwrap().default_ref, "main");
    }

    #[test]
    fn unknown_root_is_reported() {
        let manager = RootsManager::load(None, "/tmp/nowhere").unwrap();
        assert!(matches!(
            manager.get("missing"),
            Err(RootsError::UnknownRoot(_))
        ));
    }
}
