//! `den reindex` CLI (ADR-0038 Phase 5): operator-facing whole-Bear recall reconcile.
//!
//! Reconciles the derived Qdrant recall index against canonical SQLite heads for one Bear or all
//! Bears, synchronously and with immediate per-Bear counts — useful after enabling `QDRANT_URL`,
//! changing chunking/policy, or recovering a drifted index. Mirrors the `recall_index` reflection
//! lane but bypasses the queue so it works in a one-shot container without `RUN_WORKERS`.

use anyhow::{anyhow, bail, Context};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use crate::config::Config;

/// What to reindex.
pub enum ReindexTarget {
    /// Every Bear in the control plane.
    All,
    /// A single Bear by id.
    Bear(Uuid),
}

/// Parse `den reindex` arguments (`--bear <uuid>` | `--all`), where `args[0]` is `reindex`.
pub fn parse_args(args: &[String]) -> anyhow::Result<Option<ReindexTarget>> {
    let mut target: Option<ReindexTarget> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--bear" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("--bear requires a UUID"))?;
                let id =
                    Uuid::parse_str(raw).with_context(|| format!("invalid bear id {raw:?}"))?;
                target = Some(ReindexTarget::Bear(id));
                i += 2;
            }
            "--all" => {
                target = Some(ReindexTarget::All);
                i += 1;
            }
            "--help" | "-h" => {
                println!("Usage: den reindex (--bear <uuid> | --all)");
                return Ok(None);
            }
            other => bail!("unknown reindex argument {other:?}"),
        }
    }
    target
        .map(Some)
        .ok_or_else(|| anyhow!("reindex requires --bear <uuid> or --all"))
}

/// Reconcile the recall index for the requested target(s), printing per-Bear counts.
pub async fn run_reindex(target: ReindexTarget) -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::load();
    if config.qdrant_url.is_none() {
        bail!(
            "recall is disabled: set QDRANT_URL to enable the derived recall index before reindexing"
        );
    }

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&config.database_url)
        .await
        .context("connect to Postgres (DATABASE_URL)")?;
    // Sanctioned construction: `den reindex` is a short-lived CLI process, so
    // this is its one process-local `MemoryStoreManager` (ADR-0031 write
    // topology: one owning process per Bear database).
    let stores = den_memory::MemoryStoreManager::new(&config);

    let bear_ids: Vec<Uuid> = match target {
        ReindexTarget::Bear(id) => vec![id],
        ReindexTarget::All => den_service::bears::db::list_bears(&pool)
            .await
            .context("list bears")?
            .into_iter()
            .map(|bear| bear.id)
            .collect(),
    };
    if bear_ids.is_empty() {
        println!("reindex: no bears to process");
        return Ok(());
    }

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    for bear_id in bear_ids {
        match den_runtime::recall::reindex_bear_now(&pool, &config, &stores, bear_id).await {
            Ok(outcome) => {
                succeeded += 1;
                println!(
                    "reindexed bear={bear_id} indexed={} embedded={} reused={} removed_records={} removed_points={}",
                    outcome.indexed_records,
                    outcome.embedded_chunks,
                    outcome.reused_chunks,
                    outcome.removed_records,
                    outcome.removed_points,
                );
            }
            Err(error) => {
                failed += 1;
                eprintln!("reindex bear={bear_id} failed: {error}");
            }
        }
    }
    println!("reindex complete: {succeeded} ok, {failed} failed");
    if failed > 0 {
        bail!("{failed} bear(s) failed to reindex");
    }
    Ok(())
}
