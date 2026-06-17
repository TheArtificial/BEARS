//! `den import-memfs` CLI: operator-facing MemFS bundle import into per-Bear SQLite.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};
use uuid::Uuid;

use crate::config::Config;

pub struct ImportMemfsArgs {
    pub bear_id: Uuid,
    pub bundle_path: PathBuf,
    pub dry_run: bool,
}

pub fn parse_args(args: &[String]) -> anyhow::Result<ImportMemfsArgs> {
    let mut bear_id: Option<Uuid> = None;
    let mut bundle_path: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--bear" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--bear requires a UUID"))?;
                bear_id =
                    Some(Uuid::parse_str(raw).with_context(|| format!("invalid bear id {raw:?}"))?);
                i += 2;
            }
            "--bundle" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--bundle requires a path"))?;
                bundle_path = Some(PathBuf::from(raw));
                i += 2;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--help" | "-h" => {
                println!("Usage: den import-memfs --bear <uuid> --bundle <path> [--dry-run]");
                std::process::exit(0);
            }
            other => bail!("unknown import-memfs argument {other:?}"),
        }
    }

    Ok(ImportMemfsArgs {
        bear_id: bear_id.ok_or_else(|| anyhow!("import-memfs requires --bear <uuid>"))?,
        bundle_path: bundle_path.ok_or_else(|| anyhow!("import-memfs requires --bundle <path>"))?,
        dry_run,
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

    let report = den_runtime::memory::import_memfs_bundle(
        &store,
        &args.bundle_path,
        &den_runtime::memory::MemfsImportOptions {
            dry_run: args.dry_run,
            include_workflow_artifacts: false,
        },
    )
    .await
    .context("import memfs bundle")?;

    println!(
        "{}",
        serde_json::to_string_pretty(&report).context("serialize import report")?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_import_memfs_args() {
        let args = vec![
            "import-memfs".to_string(),
            "--bear".to_string(),
            "00000000-0000-0000-0000-000000000123".to_string(),
            "--bundle".to_string(),
            "/tmp/sample.bundle".to_string(),
            "--dry-run".to_string(),
        ];
        let parsed = parse_args(&args).expect("parse args");
        assert_eq!(
            parsed.bear_id,
            Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap()
        );
        assert_eq!(parsed.bundle_path, PathBuf::from("/tmp/sample.bundle"));
        assert!(parsed.dry_run);
    }
}
