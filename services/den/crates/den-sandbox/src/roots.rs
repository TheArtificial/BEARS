//! Syncable workspace roots.
//!
//! A root is either a plain directory on the sandbox host (copied per
//! sandbox) or a git upstream the provider mirrors as a **pristine,
//! server-managed bare clone** — never a human-edited working tree. Sync is
//! fetch/fast-forward only; a non-fast-forward upstream is reported, never
//! forced.
//!
//! Roots and the image catalog are Den-managed: Den pushes the full set to
//! `PUT /sandbox/v1/managed-config` and the provider persists it under the
//! workspaces volume (see [`crate::managed`]) so provisioning works between
//! pushes and across restarts. Credential material lives in 0600 files on
//! this host, referenced by path — never in the persisted config, in logs,
//! or on command lines.

use crate::proc::{run_command, CommandSpec};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SCRATCH_ROOT_NAME: &str = "scratch";
const SCRATCH_SOURCE_DIR: &str = "scratch-source";
const GIT_TIMEOUT: Duration = Duration::from_secs(300);
const GIT_OUTPUT_CAP: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RootsError {
    #[error("managed config {path}: {source}")]
    ConfigRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("managed config {path}: {source}")]
    ConfigParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid managed config: {0}")]
    InvalidManagedConfig(String),
    #[error("managed config persistence: {0}")]
    ManagedPersist(String),
    #[error("unknown root '{0}'")]
    UnknownRoot(String),
    #[error("unknown image '{name}'; catalog: {available}")]
    UnknownImage { name: String, available: String },
    #[error("root '{name}' has no upstream; publish is only supported for git-backed roots")]
    PublishUnsupported { name: String },
    #[error(
        "branch '{branch}' is the root's default ref; pushing to it requires allow_default_ref"
    )]
    DefaultRefRefused { branch: String },
    #[error("root '{name}': {detail}")]
    Git { name: String, detail: String },
    #[error("root '{name}': {detail}")]
    Workspace { name: String, detail: String },
    #[error(transparent)]
    Proc(#[from] crate::proc::ProcError),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogImageSpec {
    pub name: String,
    /// Container image reference (local tag or registry reference).
    pub image: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Catalog-wide default when neither the request nor the root names one.
    #[serde(default)]
    pub default: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncableRoot {
    pub name: String,
    /// For upstream-less roots: source directory on this host, copied per
    /// sandbox. Ignored when `upstream` is set (the pristine clone is the source).
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub upstream: Option<GitUpstream>,
    /// Catalog image name this root's sandboxes default to.
    #[serde(default)]
    pub default_image: Option<String>,
    /// Exact HTTPS hosts this root's restricted sandboxes may reach.
    #[serde(default)]
    pub allowed_outbound_hosts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

/// Where a root's upstream credential comes from. The secret itself never
/// appears in the persisted config, in logs, or on command lines.
/// JSON shapes: `{"token_env": "SITE_GIT_TOKEN"}`,
/// `{"ssh_key_path": "/etc/bears/keys/den"}`, or
/// `{"token_path": "/var/lib/bears/.../credentials/site.token"}`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RootCredential {
    /// Name of an env var (on the sandbox host) holding an HTTPS token.
    TokenEnv { token_env: String },
    /// Path to an SSH private key on the sandbox host.
    SshKeyPath { ssh_key_path: String },
    /// Path to a 0600 file on the sandbox host holding an HTTPS token
    /// (written by the managed-config sync).
    TokenPath { token_path: String },
}

#[derive(Clone)]
pub struct RootsManager {
    roots: BTreeMap<String, SyncableRoot>,
    images: Vec<CatalogImageSpec>,
    workspaces_dir: PathBuf,
    /// Version of the applied managed config (Den's content hash).
    managed_version: Option<String>,
}

impl RootsManager {
    fn insert_builtin_roots(&mut self) {
        let scratch_path = self.workspaces_dir.join(SCRATCH_SOURCE_DIR);
        if let Err(err) = std::fs::create_dir_all(&scratch_path) {
            tracing::warn!(
                path = %scratch_path.display(),
                error = %err,
                "failed to create scratch root source directory"
            );
        }
        // ponytail: one built-in scratch root is enough for now; if scratch
        // needs variants later, make built-ins data-driven instead.
        self.roots.insert(
            SCRATCH_ROOT_NAME.to_string(),
            SyncableRoot {
                name: SCRATCH_ROOT_NAME.to_string(),
                path: Some(scratch_path.to_string_lossy().into_owned()),
                upstream: None,
                default_image: None,
                allowed_outbound_hosts: Vec::new(),
            },
        );
    }

    /// Load the manager from the persisted managed config under
    /// `workspaces_dir`, starting empty when none has been pushed yet.
    pub fn load(workspaces_dir: &str) -> Result<Self, RootsError> {
        let mut manager = Self {
            roots: BTreeMap::new(),
            images: Vec::new(),
            workspaces_dir: PathBuf::from(workspaces_dir),
            managed_version: None,
        };
        if let Some(persisted) = crate::managed::load(&manager.workspaces_dir)? {
            manager.apply_managed(persisted.roots, persisted.images, persisted.version);
        } else {
            manager.insert_builtin_roots();
        }
        Ok(manager)
    }

    /// Replace the managed root/catalog set (declarative full-set semantics).
    pub fn apply_managed(
        &mut self,
        roots: Vec<SyncableRoot>,
        images: Vec<CatalogImageSpec>,
        version: Option<String>,
    ) {
        self.roots = roots
            .into_iter()
            .map(|root| (root.name.clone(), root))
            .collect();
        self.insert_builtin_roots();
        self.images = images;
        self.managed_version = version;
    }

    pub fn managed_version(&self) -> Option<&str> {
        self.managed_version.as_deref()
    }

    pub fn names(&self) -> Vec<String> {
        self.roots.keys().cloned().collect()
    }

    pub fn images(&self) -> &[CatalogImageSpec] {
        &self.images
    }

    /// Resolve a requested catalog image **name** to a container image
    /// reference: request → root default → catalog default → `fallback`
    /// (`SANDBOX_IMAGE`). Unknown names are rejected, never passed through —
    /// the catalog is the trust boundary for what can run on this host.
    pub fn resolve_image(
        &self,
        requested: Option<&str>,
        root: &SyncableRoot,
        fallback: &str,
    ) -> Result<String, RootsError> {
        let by_name = |name: &str| -> Result<String, RootsError> {
            self.images
                .iter()
                .find(|image| image.name == name)
                .map(|image| image.image.clone())
                .ok_or_else(|| RootsError::UnknownImage {
                    name: name.to_string(),
                    available: self
                        .images
                        .iter()
                        .map(|image| image.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                })
        };
        if let Some(name) = requested.map(str::trim).filter(|name| !name.is_empty()) {
            return by_name(name);
        }
        if let Some(name) = root.default_image.as_deref() {
            return by_name(name);
        }
        if let Some(image) = self.images.iter().find(|image| image.default) {
            return Ok(image.image.clone());
        }
        Ok(fallback.to_string())
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
                None => (
                    false,
                    Some("root has neither path nor upstream".to_string()),
                ),
            }
        }
    }

    pub async fn inspect_root(
        &self,
        root: &SyncableRoot,
    ) -> Result<crate::protocol::RootInspectionResponse, RootsError> {
        let upstream = root
            .upstream
            .as_ref()
            .ok_or_else(|| RootsError::Workspace {
                name: root.name.clone(),
                detail: "root has no Git upstream".to_string(),
            })?;
        let pristine = self.pristine_dir(root);
        if !pristine.is_dir() {
            return Err(RootsError::Workspace {
                name: root.name.clone(),
                detail: "pristine clone is not prepared".to_string(),
            });
        }
        let env = credential_env(root, upstream)?;
        let head = self
            .resolve_commit(root, &pristine, &env, &upstream.default_ref)
            .await?;
        let subject = self
            .git(
                root,
                Some(&pristine),
                &[],
                &["show", "-s", "--format=%s", &head],
            )
            .await?
            .trim()
            .to_string();
        let numstat = self
            .git(
                root,
                Some(&pristine),
                &[],
                &[
                    "diff-tree",
                    "--root",
                    "--no-commit-id",
                    "--numstat",
                    "-r",
                    &head,
                ],
            )
            .await?;
        let mut additions = 0_u64;
        let mut deletions = 0_u64;
        let files = numstat
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(3, '\t');
                let added = fields.next()?;
                let deleted = fields.next()?;
                let path = fields.next()?.to_string();
                let added = added.parse::<u64>().ok();
                let deleted = deleted.parse::<u64>().ok();
                additions = additions.saturating_add(added.unwrap_or(0));
                deletions = deletions.saturating_add(deleted.unwrap_or(0));
                Some(crate::protocol::RootCommitFileChange {
                    path,
                    additions: added,
                    deletions: deleted,
                })
            })
            .collect();
        let remote_ref = if upstream.default_ref.starts_with("refs/") {
            upstream.default_ref.clone()
        } else {
            format!("refs/heads/{}", upstream.default_ref)
        };
        let remote_head = self
            .git(
                root,
                None,
                &env,
                &["ls-remote", "--exit-code", &upstream.url, &remote_ref],
            )
            .await
            .ok()
            .and_then(|output| output.split_whitespace().next().map(str::to_string));
        let origin_status = match remote_head.as_deref() {
            Some(remote) if remote == head => "in_sync",
            Some(_) => "remote_differs",
            None => "remote_unavailable",
        }
        .to_string();
        let short_head = head.chars().take(8).collect();
        Ok(crate::protocol::RootInspectionResponse {
            name: root.name.clone(),
            default_ref: upstream.default_ref.clone(),
            head,
            short_head,
            subject,
            files,
            additions,
            deletions,
            remote_head,
            origin_status,
        })
    }

    pub async fn compare_root(
        &self,
        root: &SyncableRoot,
        base_ref: &str,
        head_ref: &str,
    ) -> Result<crate::protocol::RootComparisonResponse, RootsError> {
        let pristine = self.pristine_dir(root);
        if !pristine.is_dir() {
            return Err(RootsError::Workspace {
                name: root.name.clone(),
                detail: "pristine clone is not prepared".to_string(),
            });
        }
        let env = root
            .upstream
            .as_ref()
            .map(|upstream| credential_env(root, upstream))
            .transpose()?
            .unwrap_or_default();
        let base = self.resolve_commit(root, &pristine, &env, base_ref).await?;
        let head = self.resolve_commit(root, &pristine, &env, head_ref).await?;
        let patch = self
            .git(
                root,
                Some(&pristine),
                &env,
                &["diff", "--no-ext-diff", "--find-renames", &base, &head],
            )
            .await?;
        Ok(crate::protocol::RootComparisonResponse {
            base_ref: base_ref.to_string(),
            head_ref: head_ref.to_string(),
            base_commit: Some(base),
            head_commit: Some(head),
            patch_truncated: patch.len() >= GIT_OUTPUT_CAP,
            patch,
            worktree_clean: true,
        })
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
            .resolve_commit(root, &pristine, &env, &upstream.default_ref)
            .await?;
        Ok(Some(head))
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
            // Resolve in the pristine bare mirror, where every fetched branch
            // is a local refs/heads/* ref. In a normal clone, a non-HEAD branch
            // may exist only as origin/<name>, so resolving the same short name
            // there can fail even after a successful root readiness check.
            let commit = match self.resolve_commit(root, &pristine, &[], reference).await {
                Ok(commit) => commit,
                // A requested ref that does not exist yet (e.g. a job work
                // branch before its first publish) falls back to the default
                // ref instead of failing the provision.
                Err(_) if reference != upstream.default_ref => {
                    tracing::info!(
                        root = %root.name,
                        requested = reference,
                        fallback = %upstream.default_ref,
                        "requested ref missing; provisioning at the default ref"
                    );
                    self.resolve_commit(root, &pristine, &[], &upstream.default_ref)
                        .await
                        .inspect_err(|_| {
                            let _ = std::fs::remove_dir_all(&workspace);
                        })?
                }
                Err(err) => {
                    let _ = std::fs::remove_dir_all(&workspace);
                    return Err(err);
                }
            };
            self.git(
                root,
                Some(&workspace),
                &[],
                &["checkout", "--detach", &commit],
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

    /// Push a sandbox workspace's commits to the root's upstream branch.
    ///
    /// The push runs host-side with the root's credentials — they never enter
    /// the sandbox. When `auto_commit_leftovers` is set, uncommitted workspace
    /// changes are committed first (as the Den work identity) so nothing the
    /// run produced is silently dropped. `base_commit` is the commit the
    /// workspace was provisioned at (recorded on the work surface); it bounds
    /// the pushed-commit count and the nothing-to-push check.
    pub async fn publish_workspace(
        &self,
        root: &SyncableRoot,
        workspace: &Path,
        request: &crate::protocol::PublishRequest,
        base_commit: Option<&str>,
    ) -> Result<crate::protocol::PublishResponse, RootsError> {
        let Some(upstream) = &root.upstream else {
            return Err(RootsError::PublishUnsupported {
                name: root.name.clone(),
            });
        };
        let branch = request.branch.trim();
        if branch.is_empty() || branch.contains(|c: char| c.is_whitespace()) {
            return Err(RootsError::Workspace {
                name: root.name.clone(),
                detail: format!("invalid publish branch '{branch}'"),
            });
        }
        if branch == upstream.default_ref && !request.allow_default_ref {
            return Err(RootsError::DefaultRefRefused {
                branch: branch.to_string(),
            });
        }

        let mut auto_committed = false;
        if request.auto_commit_leftovers {
            let status = self
                .git(root, Some(workspace), &[], &["status", "--porcelain"])
                .await?;
            if !status.trim().is_empty() {
                self.git(root, Some(workspace), &[], &["add", "-A"]).await?;
                let label = request.run_label.as_deref().unwrap_or("unknown");
                let author_name = request
                    .author_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Den Work");
                self.git(
                    root,
                    Some(workspace),
                    &[],
                    &[
                        "-c",
                        &format!("user.name={author_name}"),
                        "-c",
                        "user.email=work@den.invalid",
                        "commit",
                        "-m",
                        &format!("work run {label}: uncommitted changes"),
                    ],
                )
                .await?;
                auto_committed = true;
            }
        }

        let commit = self
            .git(root, Some(workspace), &[], &["rev-parse", "HEAD"])
            .await?
            .trim()
            .to_string();

        // Commits ahead of the provisioned base. Without a recorded base
        // (adopted sandbox), fall back to "did HEAD move at all" = 0 ahead
        // means nothing to push only when base is known.
        let commits_pushed = match base_commit.map(str::trim).filter(|c| !c.is_empty()) {
            Some(base) => self
                .git(
                    root,
                    Some(workspace),
                    &[],
                    &["rev-list", "--count", &format!("{base}..HEAD")],
                )
                .await?
                .trim()
                .parse::<u64>()
                .unwrap_or(0),
            None => 1,
        };
        if commits_pushed == 0 {
            return Ok(crate::protocol::PublishResponse {
                branch: branch.to_string(),
                commit,
                commits_pushed: 0,
                auto_committed,
                pushed: false,
            });
        }

        let env = credential_env(root, upstream)?;
        self.git(
            root,
            Some(workspace),
            &env,
            &["push", &upstream.url, &format!("HEAD:refs/heads/{branch}")],
        )
        .await?;

        // Refresh the pristine mirror so the pushed branch is visible to the
        // next provisioning without waiting for its pre-provision sync.
        if let Err(err) = self.sync_root(root).await {
            tracing::warn!(root = %root.name, error = %err, "post-publish pristine sync failed");
        }

        Ok(crate::protocol::PublishResponse {
            branch: branch.to_string(),
            commit,
            commits_pushed,
            auto_committed,
            pushed: true,
        })
    }

    pub async fn remove_workspace(&self, sandbox_id: &str) -> Result<(), std::io::Error> {
        let workspace = self.workspace_dir(sandbox_id);
        if workspace.exists() {
            tokio::fs::remove_dir_all(&workspace).await?;
        }
        Ok(())
    }

    async fn resolve_commit(
        &self,
        root: &SyncableRoot,
        repository: &Path,
        env: &[(String, String)],
        reference: &str,
    ) -> Result<String, RootsError> {
        let expression = format!("{reference}^{{commit}}");
        let resolved = match self
            .git(
                root,
                Some(repository),
                env,
                &["rev-parse", "--verify", "--end-of-options", &expression],
            )
            .await
        {
            Ok(resolved) => resolved,
            Err(RootsError::Git { name, detail }) => {
                return Err(RootsError::Git {
                    name,
                    detail: format!(
                        "ref {reference:?} does not resolve to a commit; verify the surface default ref and that the upstream repository is not empty ({detail})"
                    ),
                });
            }
            Err(err) => return Err(err),
        };
        let oid = resolved.trim();
        if !matches!(oid.len(), 40 | 64) || !oid.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(RootsError::Git {
                name: root.name.clone(),
                detail: format!("ref {reference:?} resolved to an invalid commit object id"),
            });
        }
        Ok(oid.to_string())
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
        Some(RootCredential::TokenPath { token_path }) => {
            let token = std::fs::read_to_string(token_path).map_err(|err| RootsError::Git {
                name: root.name.clone(),
                detail: format!("read credential file {token_path}: {err}"),
            })?;
            let token = token.trim().to_string();
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
    use crate::protocol::PublishRequest;

    #[test]
    fn credential_shapes_deserialize() {
        // The persisted managed config uses these untagged shapes; changing
        // them breaks configs already on provider volumes.
        let ssh: RootCredential =
            serde_json::from_str(r#"{"ssh_key_path": "/etc/bears/keys/den"}"#).unwrap();
        assert!(matches!(ssh, RootCredential::SshKeyPath { .. }));
        let env: RootCredential =
            serde_json::from_str(r#"{"token_env": "SITE_GIT_TOKEN"}"#).unwrap();
        assert!(matches!(env, RootCredential::TokenEnv { .. }));
        let path: RootCredential =
            serde_json::from_str(r#"{"token_path": "/var/lib/x/site.token"}"#).unwrap();
        assert!(matches!(path, RootCredential::TokenPath { .. }));
    }

    #[test]
    fn token_path_credential_reads_file() {
        let tmp = std::env::temp_dir().join(format!(
            "den-sbx-tokenpath-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let token_file = tmp.join("site.token");
        std::fs::write(&token_file, "sekrit-token\n").unwrap();
        let root = SyncableRoot {
            name: "site".to_string(),
            path: None,
            upstream: Some(GitUpstream {
                url: "https://example.invalid/site.git".to_string(),
                default_ref: "main".to_string(),
                credential: Some(RootCredential::TokenPath {
                    token_path: token_file.to_string_lossy().into_owned(),
                }),
            }),
            default_image: None,
            allowed_outbound_hosts: Vec::new(),
        };
        let upstream = root.upstream.as_ref().unwrap();
        let env = credential_env(&root, upstream).unwrap();
        let helper = env
            .iter()
            .find(|(k, _)| k == "GIT_CONFIG_VALUE_0")
            .map(|(_, v)| v.as_str())
            .expect("credential helper env");
        assert!(helper.contains("password=sekrit-token"), "trimmed token");

        // Missing file fails fast with the path, not a prompt.
        let missing = SyncableRoot {
            upstream: Some(GitUpstream {
                credential: Some(RootCredential::TokenPath {
                    token_path: tmp.join("gone.token").to_string_lossy().into_owned(),
                }),
                ..root.upstream.clone().unwrap()
            }),
            ..root.clone()
        };
        let err = credential_env(&missing, missing.upstream.as_ref().unwrap()).unwrap_err();
        assert!(err.to_string().contains("gone.token"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unknown_root_is_reported() {
        let manager = RootsManager::load("/tmp/nowhere").unwrap();
        assert!(matches!(
            manager.get("missing"),
            Err(RootsError::UnknownRoot(_))
        ));
    }

    #[test]
    fn apply_managed_replaces_the_full_set() {
        let mut manager = RootsManager::load("/tmp/nowhere").unwrap();
        manager.apply_managed(
            vec![plain_root("a"), plain_root("b")],
            vec![CatalogImageSpec {
                name: "base".into(),
                image: "bears/sandbox:latest".into(),
                description: None,
                default: true,
            }],
            Some("v1".to_string()),
        );
        assert_eq!(manager.names(), ["a", "b", SCRATCH_ROOT_NAME]);
        assert_eq!(manager.images().len(), 1);
        assert_eq!(manager.managed_version(), Some("v1"));
        manager.apply_managed(vec![plain_root("c")], Vec::new(), Some("v2".to_string()));
        assert_eq!(manager.names(), ["c", SCRATCH_ROOT_NAME]);
        assert!(manager.get("a").is_err());
        assert!(manager.get(SCRATCH_ROOT_NAME).is_ok());
        assert!(manager.images().is_empty());
        assert_eq!(manager.managed_version(), Some("v2"));
    }

    fn manager_with_images(images: Vec<CatalogImageSpec>) -> RootsManager {
        RootsManager {
            roots: BTreeMap::new(),
            images,
            workspaces_dir: PathBuf::from("/tmp/nowhere"),
            managed_version: None,
        }
    }

    fn plain_root(name: &str) -> SyncableRoot {
        SyncableRoot {
            name: name.to_string(),
            path: Some("/srv/x".to_string()),
            upstream: None,
            default_image: None,
            allowed_outbound_hosts: Vec::new(),
        }
    }

    #[test]
    fn image_resolution_order_and_unknown_rejection() {
        let manager = manager_with_images(vec![
            CatalogImageSpec {
                name: "base".into(),
                image: "bears/sandbox:latest".into(),
                description: None,
                default: true,
            },
            CatalogImageSpec {
                name: "rust".into(),
                image: "bears/sandbox-rust:latest".into(),
                description: None,
                default: false,
            },
        ]);
        let mut root = plain_root("r");

        // Request wins.
        assert_eq!(
            manager
                .resolve_image(Some("rust"), &root, "fallback")
                .unwrap(),
            "bears/sandbox-rust:latest"
        );
        // Root default next.
        root.default_image = Some("rust".to_string());
        assert_eq!(
            manager.resolve_image(None, &root, "fallback").unwrap(),
            "bears/sandbox-rust:latest"
        );
        // Catalog default next.
        root.default_image = None;
        assert_eq!(
            manager.resolve_image(None, &root, "fallback").unwrap(),
            "bears/sandbox:latest"
        );
        // Unknown names are rejected, never passed through as references.
        assert!(matches!(
            manager.resolve_image(Some("bears/sandbox:latest"), &root, ""),
            Err(RootsError::UnknownImage { .. })
        ));

        // Empty catalog falls back to the configured default reference.
        let empty = manager_with_images(Vec::new());
        assert_eq!(
            empty.resolve_image(None, &root, "fallback").unwrap(),
            "fallback"
        );
        assert!(matches!(
            empty.resolve_image(Some("rust"), &root, "fallback"),
            Err(RootsError::UnknownImage { .. })
        ));
    }

    // --- publish tests: real host git against a tempdir bare upstream ---

    fn sh_git(cwd: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@test.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@test.invalid")
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    struct PublishFixture {
        tmp: PathBuf,
        manager: RootsManager,
        root: SyncableRoot,
        base_commit: String,
    }

    impl Drop for PublishFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.tmp);
        }
    }

    fn publish_fixture() -> PublishFixture {
        let tmp =
            std::env::temp_dir().join(format!("den-sbx-publish-{}", uuid::Uuid::new_v4().simple()));
        let upstream = tmp.join("upstream.git");
        std::fs::create_dir_all(&upstream).unwrap();
        sh_git(&upstream, &["init", "--bare", "-b", "main", "."]);
        let seed = tmp.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        sh_git(&seed, &["init", "-b", "main", "."]);
        std::fs::write(seed.join("README.md"), "seed\n").unwrap();
        sh_git(&seed, &["add", "-A"]);
        sh_git(&seed, &["commit", "-m", "initial"]);
        sh_git(&seed, &["push", &upstream.to_string_lossy(), "main"]);
        let base_commit = sh_git(&seed, &["rev-parse", "HEAD"]).trim().to_string();

        let root = SyncableRoot {
            name: "fixture".to_string(),
            path: None,
            upstream: Some(GitUpstream {
                url: upstream.to_string_lossy().into_owned(),
                default_ref: "main".to_string(),
                credential: None,
            }),
            default_image: None,
            allowed_outbound_hosts: Vec::new(),
        };
        let manager = RootsManager {
            roots: std::iter::once(("fixture".to_string(), root.clone())).collect(),
            images: Vec::new(),
            workspaces_dir: tmp.join("workspaces"),
            managed_version: None,
        };
        PublishFixture {
            tmp,
            manager,
            root,
            base_commit,
        }
    }

    fn publish_request(branch: &str) -> PublishRequest {
        PublishRequest {
            branch: branch.to_string(),
            auto_commit_leftovers: true,
            allow_default_ref: false,
            author_name: Some("Test Bear".to_string()),
            run_label: Some("test-run".to_string()),
        }
    }

    #[tokio::test]
    async fn provision_uses_non_head_default_branch_resolved_from_pristine_mirror() {
        let mut fx = publish_fixture();
        let seed = fx.tmp.join("seed");
        let upstream = fx.tmp.join("upstream.git");
        sh_git(&seed, &["checkout", "-b", "test"]);
        std::fs::write(seed.join("TEST.md"), "test branch\n").unwrap();
        sh_git(&seed, &["add", "-A"]);
        sh_git(&seed, &["commit", "-m", "test branch"]);
        sh_git(&seed, &["push", &upstream.to_string_lossy(), "test"]);
        let test_commit = sh_git(&seed, &["rev-parse", "HEAD"]).trim().to_string();
        fx.root.upstream.as_mut().unwrap().default_ref = "test".to_string();

        assert_eq!(
            fx.manager.sync_root(&fx.root).await.unwrap().as_deref(),
            Some(test_commit.as_str())
        );
        let inspection = fx.manager.inspect_root(&fx.root).await.unwrap();
        assert_eq!(inspection.head, test_commit);
        assert_eq!(inspection.default_ref, "test");
        assert_eq!(inspection.subject, "test branch");
        assert_eq!(inspection.origin_status, "in_sync");
        assert_eq!(inspection.additions, 1);
        assert_eq!(inspection.deletions, 0);
        assert!(inspection.files.iter().any(|file| file.path == "TEST.md"));
        let workspace = fx
            .manager
            .provision_workspace(&fx.root, None, "sbx-non-head-default")
            .await
            .expect("prepared non-HEAD default branch should provision");
        assert_eq!(
            sh_git(&workspace, &["rev-parse", "HEAD"]).trim(),
            test_commit
        );
    }

    #[tokio::test]
    async fn unresolved_ref_error_names_the_ref_and_remediation() {
        let fx = publish_fixture();
        fx.manager.sync_root(&fx.root).await.unwrap();
        let err = fx
            .manager
            .resolve_commit(
                &fx.root,
                &fx.manager.pristine_dir(&fx.root),
                &[],
                "missing-branch",
            )
            .await
            .expect_err("missing ref should fail");
        let message = err.to_string();
        assert!(message.contains("missing-branch"), "{message}");
        assert!(message.contains("surface default ref"), "{message}");
        assert!(message.contains("not empty"), "{message}");
    }

    #[tokio::test]
    async fn option_like_requested_ref_falls_back_without_checkout_option_injection() {
        let fx = publish_fixture();
        fx.manager.sync_root(&fx.root).await.unwrap();
        let workspace = fx
            .manager
            .provision_workspace(&fx.root, Some("-b"), "sbx-option-ref")
            .await
            .expect("option-like ref should fall back to default");
        assert_eq!(
            sh_git(&workspace, &["rev-parse", "HEAD"]).trim(),
            fx.base_commit
        );
    }

    #[tokio::test]
    async fn publish_pushes_workspace_commits_and_auto_commits_leftovers() {
        let fx = publish_fixture();
        fx.manager.sync_root(&fx.root).await.unwrap();
        let workspace = fx
            .manager
            .provision_workspace(&fx.root, None, "sbx-pub")
            .await
            .unwrap();

        // An in-run commit (what the armature does) plus a leftover change.
        std::fs::write(workspace.join("work.txt"), "done\n").unwrap();
        sh_git(&workspace, &["add", "-A"]);
        sh_git(&workspace, &["commit", "-m", "do the task"]);
        std::fs::write(workspace.join("leftover.txt"), "oops\n").unwrap();

        let outcome = fx
            .manager
            .publish_workspace(
                &fx.root,
                &workspace,
                &publish_request("den/job-test"),
                Some(&fx.base_commit),
            )
            .await
            .unwrap();
        assert!(outcome.pushed);
        assert!(outcome.auto_committed);
        assert_eq!(outcome.commits_pushed, 2);
        assert_eq!(
            sh_git(&workspace, &["show", "-s", "--format=%an", "HEAD"]).trim(),
            "Test Bear"
        );

        // The upstream branch exists and matches the workspace head.
        let upstream = fx.tmp.join("upstream.git");
        let upstream_head = sh_git(&upstream, &["rev-parse", "den/job-test"]);
        assert_eq!(upstream_head.trim(), outcome.commit);
        // main is untouched.
        assert_eq!(
            sh_git(&upstream, &["rev-parse", "main"]).trim(),
            fx.base_commit
        );
    }

    #[tokio::test]
    async fn publish_refuses_default_ref_and_skips_empty_pushes() {
        let fx = publish_fixture();
        fx.manager.sync_root(&fx.root).await.unwrap();
        let workspace = fx
            .manager
            .provision_workspace(&fx.root, None, "sbx-empty")
            .await
            .unwrap();

        let err = fx
            .manager
            .publish_workspace(
                &fx.root,
                &workspace,
                &publish_request("main"),
                Some(&fx.base_commit),
            )
            .await
            .expect_err("default ref must be refused");
        assert!(
            matches!(err, RootsError::DefaultRefRefused { .. }),
            "{err:?}"
        );

        // Nothing beyond the base: no push, no branch created.
        let outcome = fx
            .manager
            .publish_workspace(
                &fx.root,
                &workspace,
                &publish_request("den/job-empty"),
                Some(&fx.base_commit),
            )
            .await
            .unwrap();
        assert!(!outcome.pushed);
        assert_eq!(outcome.commits_pushed, 0);
        let upstream = fx.tmp.join("upstream.git");
        let branches = sh_git(&upstream, &["branch", "--list"]);
        assert!(!branches.contains("den/job-empty"), "{branches}");
    }

    #[tokio::test]
    async fn publish_unsupported_for_upstreamless_roots() {
        let fx = publish_fixture();
        let plain = plain_root("plain");
        let err = fx
            .manager
            .publish_workspace(&plain, &fx.tmp, &publish_request("den/job-x"), None)
            .await
            .expect_err("path roots cannot publish");
        assert!(
            matches!(err, RootsError::PublishUnsupported { .. }),
            "{err:?}"
        );
    }
}
