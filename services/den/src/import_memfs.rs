//! `den import-memfs` CLI: operator-facing MemFS bundle import into per-Bear SQLite.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};
use uuid::Uuid;

use crate::config::Config;

pub struct ImportMemfsArgs {
    pub bear_id: Uuid,
    pub source: ImportMemfsCliSource,
    pub dry_run: bool,
    pub import_history: bool,
    pub include_workflow_artifacts: bool,
    pub report_path: PathBuf,
}

pub enum ImportMemfsCliSource {
    Bundle(PathBuf),
    GitDir(PathBuf),
}

pub fn parse_args(args: &[String]) -> anyhow::Result<ImportMemfsArgs> {
    let mut bear_id: Option<Uuid> = None;
    let mut bundle_path: Option<PathBuf> = None;
    let mut git_dir_path: Option<PathBuf> = None;
    let mut report_path = PathBuf::from("report.json");
    let mut dry_run = false;
    let mut import_history = false;
    let mut include_workflow_artifacts = false;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--bear" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--bear requires a UUID"))?;
                bear_id = Some(Uuid::parse_str(raw).with_context(|| format!("invalid bear id {raw:?}"))?);
                i += 2;
            }
            "--bundle" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--bundle requires a path"))?;
                bundle_path = Some(PathBuf::from(raw));
                i += 2;
            }
            "--git-dir" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--git-dir requires a path"))?;
                git_dir_path = Some(PathBuf::from(raw));
                i += 2;
            }
            "--report" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--report requires a path"))?;
                report_path = PathBuf::from(raw);
                i += 2;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--import-history" => {
                import_history = true;
                i += 1;
            }
            "--heads-only" => {
                import_history = false;
                i += 1;
            }
            "--include-workflow-artifacts" => {
                include_workflow_artifacts = true;
                i += 1;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: den import-memfs --bear <uuid> (--bundle <path> | --git-dir <path>) [--dry-run] [--import-history] [--include-workflow-artifacts] [--report <path>]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown import-memfs argument {other:?}"),
        }
    }

    let source = match (bundle_path, git_dir_path) {
        (Some(_), Some(_)) => bail!("use either --bundle or --git-dir, not both"),
        (Some(path), None) => ImportMemfsCliSource::Bundle(path),
        (None, Some(path)) => ImportMemfsCliSource::GitDir(path),
        (None, None) => bail!("import-memfs requires --bundle <path> or --git-dir <path>"),
    };

    Ok(ImportMemfsArgs {
        bear_id: bear_id.ok_or_else(|| anyhow!("import-memfs requires --bear <uuid>"))?,
        source,
        dry_run,
        import_history,
        include_workflow_artifacts,
        report_path,
    })
}

pub async fn run_import_memfs(args: ImportMemfsArgs) -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::load();
    let stores = den_runtime::memory::MemoryStoreManager::new(&config);
    let store = stores
        .store_for_bear(args.bear_id)
        .await
        .context("open per-bear sqlite store")?;

    let options = den_runtime::memory::MemfsImportOptions {
        dry_run: args.dry_run,
        include_workflow_artifacts: args.include_workflow_artifacts,
        import_history: args.import_history,
    };

    let report = match &args.source {
        ImportMemfsCliSource::Bundle(path) => {
            den_runtime::memory::import_memfs_bundle(&store, path, &options)
                .await
                .context("import memfs bundle")?
        }
        ImportMemfsCliSource::GitDir(path) => {
            den_runtime::memory::import_memfs_git_dir(&store, path, &options)
                .await
                .context("import memfs git dir")?
        }
    };

    let report_json = serde_json::to_string_pretty(&report).context("serialize import report")?;
    if let Some(parent) = args.report_path.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create report directory {}", parent.display()))?;
    }
    std::fs::write(&args.report_path, &report_json)
        .with_context(|| format!("write report {}", args.report_path.display()))?;

    println!("{report_json}");
    println!("report written to {}", args.report_path.display());
    if !args.dry_run {
        println!("post-import checklist:");
        println!("1. Review {}", args.report_path.display());
        println!("2. Spot-check Bear Admin → Memory browse / record detail");
        println!("3. Run `den reindex --bear {}` when QDRANT_URL is set", args.bear_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bundle_import_args() {
        let args = vec![
            "import-memfs".to_string(),
            "--bear".to_string(),
            "00000000-0000-0000-0000-000000000123".to_string(),
            "--bundle".to_string(),
            "/tmp/sample.bundle".to_string(),
            "--dry-run".to_string(),
            "--import-history".to_string(),
            "--report".to_string(),
            "/tmp/report.json".to_string(),
        ];
        let parsed = parse_args(&args).expect("parse args");
        assert_eq!(parsed.bear_id, Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap());
        assert!(matches!(parsed.source, ImportMemfsCliSource::Bundle(ref path) if path == &PathBuf::from("/tmp/sample.bundle")));
        assert!(parsed.dry_run);
        assert!(parsed.import_history);
        assert_eq!(parsed.report_path, PathBuf::from("/tmp/report.json"));
    }

    #[test]
    fn parses_git_dir_import_args() {
        let args = vec![
            "import-memfs".to_string(),
            "--bear".to_string(),
            "00000000-0000-0000-0000-000000000123".to_string(),
            "--git-dir".to_string(),
            "/tmp/repo.git".to_string(),
            "--include-workflow-artifacts".to_string(),
        ];
        let parsed = parse_args(&args).expect("parse args");
        assert!(matches!(parsed.source, ImportMemfsCliSource::GitDir(ref path) if path == &PathBuf::from("/tmp/repo.git")));
        assert!(parsed.include_workflow_artifacts);
        assert_eq!(parsed.report_path, PathBuf::from("report.json"));
    }
}
