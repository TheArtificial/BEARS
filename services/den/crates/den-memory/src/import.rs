use std::path::Path;
use std::process::Command;

use serde::Serialize;
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};
use uuid::Uuid;

use den_core::DenError;

use crate::{BearMemoryStore, LogicalMemoryPath};

const IMPORTED_AT_FORMAT: &str = "imported_at";

#[derive(Debug, Clone, Serialize, Default)]
pub struct MemfsImportOptions {
    pub dry_run: bool,
    pub include_workflow_artifacts: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemfsImportReport {
    pub bear_id: String,
    pub bundle_path: String,
    pub dry_run: bool,
    pub imported_count: usize,
    pub skipped_count: usize,
    pub quarantined_count: usize,
    pub branch_reports: Vec<MemfsBranchReport>,
    pub imported_paths_sample: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemfsBranchReport {
    pub branch: String,
    pub imported_count: usize,
    pub skipped_count: usize,
    pub quarantined_count: usize,
}

#[derive(Debug, Clone)]
struct ImportDraft {
    memory_id: String,
    logical: LogicalMemoryPath,
    logical_path: String,
    kind: String,
    author_profile: String,
    created_at: String,
    content_text: String,
    metadata_json: Value,
}

#[derive(Debug, Clone)]
struct BranchMapping {
    author_profile: &'static str,
}

pub async fn import_memfs_bundle(
    store: &BearMemoryStore,
    bundle_path: &Path,
    options: &MemfsImportOptions,
) -> Result<MemfsImportReport, DenError> {
    if !bundle_path.exists() {
        return Err(DenError::NotFound(format!(
            "bundle not found: {}",
            bundle_path.display()
        )));
    }

    let temp_repo = std::env::temp_dir().join(format!("den-memfs-import-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_repo)
        .map_err(|err| DenError::System(format!("create temp import dir failed: {err}")))?;

    let import_result = import_memfs_bundle_inner(store, bundle_path, &temp_repo, options).await;
    if let Err(err) = std::fs::remove_dir_all(&temp_repo) {
        tracing::warn!(path = %temp_repo.display(), error = %err, "cleanup temp memfs import dir failed");
    }
    import_result
}

async fn import_memfs_bundle_inner(
    store: &BearMemoryStore,
    bundle_path: &Path,
    temp_repo: &Path,
    options: &MemfsImportOptions,
) -> Result<MemfsImportReport, DenError> {
    git(None, &["bundle", "verify"], Some(bundle_path))?;
    git(None, &["init", "--quiet"], Some(temp_repo))?;

    let bundle_str = bundle_path.to_string_lossy().to_string();
    git(
        Some(temp_repo),
        &["fetch", "--quiet", &bundle_str, "refs/heads/*:refs/heads/*"],
        None,
    )?;

    let known_branches = ordered_known_branches();
    let discovered = git(
        Some(temp_repo),
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        None,
    )?;
    let mut branches = discovered
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    branches.sort();

    let mut ordered = known_branches
        .into_iter()
        .filter(|name| branches.iter().any(|branch| branch == name))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    for branch in branches {
        if !ordered.iter().any(|known| known == &branch) {
            ordered.push(branch);
        }
    }

    let import_run_id = Uuid::new_v4().to_string();
    let imported_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|err| DenError::System(format!("format import timestamp failed: {err}")))?;

    let mut branch_reports = Vec::new();
    let mut imported_count = 0usize;
    let mut skipped_count = 0usize;
    let mut quarantined_count = 0usize;
    let mut imported_paths_sample = Vec::new();

    for branch in ordered {
        let paths_output = git(
            Some(temp_repo),
            &["ls-tree", "-r", "--name-only", &branch],
            None,
        )?;
        let mut branch_imported = 0usize;
        let mut branch_skipped = 0usize;
        let mut branch_quarantined = 0usize;

        for raw_path in paths_output
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let Some(normalized_path) = normalize_memfs_path(raw_path) else {
                branch_skipped += 1;
                continue;
            };

            let Some(mapping) = branch_mapping(&branch, &normalized_path) else {
                branch_quarantined += 1;
                continue;
            };

            if !options.include_workflow_artifacts && is_workflow_artifact(&normalized_path) {
                branch_skipped += 1;
                continue;
            }

            let last_commit = git(
                Some(temp_repo),
                &["log", "-1", "--format=%H", &branch, "--", &normalized_path],
                None,
            )?;
            let commit = last_commit.trim();
            if commit.is_empty() {
                branch_skipped += 1;
                continue;
            }

            let blob_sha = git(
                Some(temp_repo),
                &["rev-parse", &format!("{branch}:{normalized_path}")],
                None,
            )?;
            let blob_sha = blob_sha.trim();
            if blob_sha.is_empty() {
                branch_skipped += 1;
                continue;
            }

            let created_at = normalize_git_timestamp(
                git(
                    Some(temp_repo),
                    &["log", "-1", "--format=%aI", &branch, "--", &normalized_path],
                    None,
                )?
                .trim(),
            )?;

            let content_bytes = git_bytes(
                Some(temp_repo),
                &["show", &format!("{branch}:{normalized_path}")],
                None,
            )?;
            if content_bytes.is_empty() {
                branch_skipped += 1;
                continue;
            }
            let content_text = match String::from_utf8(content_bytes) {
                Ok(value) => value,
                Err(_) => {
                    branch_skipped += 1;
                    continue;
                }
            };
            if content_text.trim().is_empty() {
                branch_skipped += 1;
                continue;
            }

            let logical = LogicalMemoryPath::from_logical_path(&normalized_path);
            let logical_path = logical.to_logical_path();
            let (kind, inferred_kind) = infer_kind(&logical_path, &logical);
            let memory_id = deterministic_import_memory_id(&branch, &logical_path, commit);
            let metadata_json = json!({
                "memfs_import": {
                    "branch": branch,
                    "path": normalized_path,
                    "commit": commit,
                    "blob_sha": blob_sha,
                    "import_run_id": import_run_id,
                    IMPORTED_AT_FORMAT: imported_at,
                    "inferred_kind": inferred_kind
                }
            });

            let draft = ImportDraft {
                memory_id,
                logical,
                logical_path,
                kind,
                author_profile: mapping.author_profile.to_string(),
                created_at,
                content_text,
                metadata_json,
            };

            if options.dry_run {
                branch_imported += 1;
                if imported_paths_sample.len() < 20 {
                    imported_paths_sample.push(draft.logical_path.clone());
                }
                continue;
            }

            let inserted = insert_import_draft(store, &draft).await?;
            if inserted {
                branch_imported += 1;
                if imported_paths_sample.len() < 20 {
                    imported_paths_sample.push(draft.logical_path.clone());
                }
            } else {
                branch_skipped += 1;
            }
        }

        imported_count += branch_imported;
        skipped_count += branch_skipped;
        quarantined_count += branch_quarantined;
        branch_reports.push(MemfsBranchReport {
            branch,
            imported_count: branch_imported,
            skipped_count: branch_skipped,
            quarantined_count: branch_quarantined,
        });
    }

    Ok(MemfsImportReport {
        bear_id: store.bear_id().to_string(),
        bundle_path: bundle_path.display().to_string(),
        dry_run: options.dry_run,
        imported_count,
        skipped_count,
        quarantined_count,
        branch_reports,
        imported_paths_sample,
    })
}

async fn insert_import_draft(
    store: &BearMemoryStore,
    draft: &ImportDraft,
) -> Result<bool, DenError> {
    let sequence_no = store.next_sequence().await?;
    let rows = sqlx::query(
        r"
        INSERT OR IGNORE INTO memory_records (
            memory_id, bear_id, sequence_no, scope_type, scope_profile, kind,
            author_profile, author_agent_id, created_at, content_text, metadata_json,
            visibility, logical_path, work_surface_ref, valid_from
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'normal', ?, ?, ?)
        ",
    )
    .bind(&draft.memory_id)
    .bind(store.bear_id().to_string())
    .bind(sequence_no)
    .bind(draft.logical.scope_type.as_str())
    .bind(&draft.logical.scope_profile)
    .bind(&draft.kind)
    .bind(&draft.author_profile)
    .bind(Option::<String>::None)
    .bind(&draft.created_at)
    .bind(&draft.content_text)
    .bind(draft.metadata_json.to_string())
    .bind(&draft.logical_path)
    .bind(&draft.logical.work_surface_ref)
    .bind(&draft.created_at)
    .execute(store.pool())
    .await
    .map_err(|err| DenError::System(format!("insert imported memory record failed: {err}")))?
    .rows_affected();
    Ok(rows > 0)
}

fn normalize_git_timestamp(raw: &str) -> Result<String, DenError> {
    let parsed = OffsetDateTime::parse(raw, &Rfc3339)
        .map_err(|err| DenError::Parsing(format!("invalid git timestamp {raw:?}: {err}")))?;
    parsed
        .to_offset(UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|err| DenError::System(format!("format normalized git timestamp failed: {err}")))
}

fn deterministic_import_memory_id(branch: &str, logical_path: &str, commit: &str) -> String {
    format!("memfs-import:{branch}:{commit}:{logical_path}")
}

fn normalize_memfs_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('/');
    if trimmed.is_empty() || trimmed == ".gitkeep" {
        return None;
    }
    let parts = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|segment| *segment == "..") {
        return None;
    }
    let normalized = parts.join("/");
    if !normalized.ends_with(".md") {
        return None;
    }
    Some(normalized)
}

fn branch_mapping(branch: &str, path: &str) -> Option<BranchMapping> {
    match branch {
        "pair" if path.starts_with("pair/") => Some(BranchMapping {
            author_profile: "pair",
        }),
        "curate" if path.starts_with("curate/") || path.starts_with("core/") => {
            Some(BranchMapping {
                author_profile: "curate",
            })
        }
        "work" if path.starts_with("work/") => Some(BranchMapping {
            author_profile: "work",
        }),
        "watch" if path.starts_with("watch/") => Some(BranchMapping {
            author_profile: "watch",
        }),
        "talk" | "chat" if path.starts_with("chat/") => Some(BranchMapping {
            author_profile: "chat",
        }),
        _ => None,
    }
}

fn is_workflow_artifact(path: &str) -> bool {
    path.contains("/tasks/") || path.contains("/results/")
}

fn infer_kind(path: &str, logical: &LogicalMemoryPath) -> (String, bool) {
    let segments = path.split('/').collect::<Vec<_>>();
    for segment in &segments {
        match *segment {
            "notes" => return ("note".to_string(), false),
            "logs" => return ("log".to_string(), false),
            "decisions" => return ("decision".to_string(), false),
            "summaries" => return ("summary".to_string(), false),
            "scratch" => return ("scratch".to_string(), false),
            "reflection" | "reflections" => return ("reflection".to_string(), false),
            _ => {}
        }
    }

    if path.contains("/work_surfaces/") {
        return (logical.kind.clone(), true);
    }

    let fallback = segments
        .last()
        .map(|name| name.trim_end_matches(".md"))
        .filter(|name| !name.is_empty())
        .unwrap_or("note");
    (fallback.to_string(), true)
}

fn ordered_known_branches() -> Vec<&'static str> {
    vec!["curate", "pair", "work", "talk", "chat", "watch"]
}

fn git(repo: Option<&Path>, args: &[&str], extra_path: Option<&Path>) -> Result<String, DenError> {
    let output = git_bytes(repo, args, extra_path)?;
    String::from_utf8(output)
        .map_err(|err| DenError::Parsing(format!("git output was not utf-8: {err}")))
}

fn git_bytes(
    repo: Option<&Path>,
    args: &[&str],
    extra_path: Option<&Path>,
) -> Result<Vec<u8>, DenError> {
    let mut command = Command::new("git");
    if let Some(repo_path) = repo {
        command.arg("-C").arg(repo_path);
    }
    for arg in args {
        command.arg(arg);
    }
    if let Some(path) = extra_path {
        command.arg(path);
    }
    let output = command
        .output()
        .map_err(|err| DenError::System(format!("spawn git failed: {err}")))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(DenError::System(format!(
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::head_record_for_logical_path;
    use crate::test_support::new_test_store;

    #[test]
    fn normalizes_and_rejects_memfs_paths() {
        assert_eq!(
            normalize_memfs_path("/pair/notes/test.md").as_deref(),
            Some("pair/notes/test.md")
        );
        assert_eq!(
            normalize_memfs_path("pair//notes///test.md").as_deref(),
            Some("pair/notes/test.md")
        );
        assert!(normalize_memfs_path("pair/notes/test.txt").is_none());
        assert!(normalize_memfs_path("pair/../secret.md").is_none());
    }

    #[test]
    fn infers_kind_from_directory_conventions() {
        let logical = LogicalMemoryPath::from_logical_path("pair/notes/incident.md");
        assert_eq!(
            infer_kind("pair/notes/incident.md", &logical),
            ("note".to_string(), false)
        );

        let surface =
            LogicalMemoryPath::from_logical_path("core/work_surfaces/app/architecture.md");
        assert_eq!(
            infer_kind("core/work_surfaces/app/architecture.md", &surface),
            ("architecture".to_string(), true)
        );
    }

    #[tokio::test]
    async fn imports_bundle_heads_only_and_is_idempotent() {
        let store = new_test_store().await;
        let temp_root =
            std::env::temp_dir().join(format!("den-memfs-import-test-{}", Uuid::new_v4()));
        let repo_dir = temp_root.join("repo");
        let bundle_path = temp_root.join("fixture.bundle");
        std::fs::create_dir_all(&repo_dir).expect("create repo dir");

        run_git(&repo_dir, &["init", "--quiet"]);
        run_git(&repo_dir, &["config", "user.email", "agent@example.com"]);
        run_git(&repo_dir, &["config", "user.name", "Agent"]);

        write_file(repo_dir.join("core/bear-overview.md"), "# Bear\n");
        run_git(&repo_dir, &["add", "."]);
        commit_all(&repo_dir, "curate base");
        run_git(&repo_dir, &["branch", "-M", "curate"]);

        write_file(repo_dir.join("core/bear-overview.md"), "# Bear v2\n");
        commit_all(&repo_dir, "curate update");

        run_git(&repo_dir, &["checkout", "-b", "pair"]);
        write_file(repo_dir.join("pair/notes/session.md"), "pair note\n");
        commit_all(&repo_dir, "pair note");

        run_git(&repo_dir, &["checkout", "-b", "talk", "curate"]);
        write_file(repo_dir.join("chat/logs/welcome.md"), "chat log\n");
        commit_all(&repo_dir, "chat log");

        run_git(&repo_dir, &["checkout", "curate"]);
        run_git(
            &repo_dir,
            &[
                "bundle",
                "create",
                bundle_path.to_str().expect("bundle path utf8"),
                "curate",
                "pair",
                "talk",
            ],
        );

        let report = import_memfs_bundle(&store, &bundle_path, &MemfsImportOptions::default())
            .await
            .expect("import bundle");
        assert_eq!(report.imported_count, 3);
        assert_eq!(report.quarantined_count, 2);
        assert!(
            head_record_for_logical_path(&store, "core/bear-overview.md")
                .await
                .expect("lookup head")
                .is_some()
        );
        let head = head_record_for_logical_path(&store, "core/bear-overview.md")
            .await
            .expect("lookup head")
            .expect("head exists");
        assert_eq!(head.content_text, "# Bear v2\n");
        assert_eq!(head.scope_type, crate::MemoryScopeType::Shared);

        let chat = head_record_for_logical_path(&store, "chat/logs/welcome.md")
            .await
            .expect("lookup chat head")
            .expect("chat head exists");
        assert_eq!(chat.scope_profile.as_deref(), Some("chat"));
        assert_eq!(chat.kind, "log");
        assert_eq!(chat.scope_type, crate::MemoryScopeType::ProfileLocal);

        let rerun = import_memfs_bundle(&store, &bundle_path, &MemfsImportOptions::default())
            .await
            .expect("reimport bundle");
        assert_eq!(rerun.imported_count, 0);
        assert_eq!(rerun.skipped_count, 3);

        std::fs::remove_dir_all(&temp_root).ok();
    }

    fn write_file(path: std::path::PathBuf, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(path, content).expect("write file");
    }

    fn commit_all(repo_dir: &Path, message: &str) {
        run_git(repo_dir, &["add", "."]);
        run_git(repo_dir, &["commit", "--quiet", "-m", message]);
    }

    fn run_git(repo_dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo_dir)
            .args(args)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {:?} failed", args);
    }
}
