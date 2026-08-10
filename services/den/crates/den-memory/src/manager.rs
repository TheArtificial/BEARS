use std::{collections::HashMap, path::PathBuf, sync::Arc};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use tokio::sync::Mutex;
use uuid::Uuid;

use den_core::{config::Config, DenError};

use super::{migrate::migrate_bear_sqlite_schema, records::BearMemoryStore};

const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Owns per-Bear SQLite pools (single writer per bear DB).
#[derive(Clone, Debug)]
pub struct MemoryStoreManager {
    data_dir: PathBuf,
    pools: Arc<Mutex<HashMap<Uuid, SqlitePool>>>,
}

impl MemoryStoreManager {
    /// Construct the process-wide manager. Per the ADR-0031 write-topology
    /// amendment there must be exactly **one** instance per owning process
    /// (cloning shares the pool map): production code receives a clone of the
    /// instance built at process startup rather than calling `new` again.
    /// Sanctioned exceptions: short-lived CLI subcommands (`reindex`,
    /// `import-legacy-memory`, `seed`) and test code.
    pub fn new(config: &Config) -> Self {
        Self {
            data_dir: PathBuf::from(&config.bear_sqlite_data_dir),
            pools: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn store_for_bear(&self, bear_id: Uuid) -> Result<BearMemoryStore, DenError> {
        let pool = self.pool_for_bear(bear_id).await?;
        Ok(BearMemoryStore::new(bear_id, pool))
    }

    async fn pool_for_bear(&self, bear_id: Uuid) -> Result<SqlitePool, DenError> {
        let mut guard = self.pools.lock().await;
        if let Some(pool) = guard.get(&bear_id) {
            return Ok(pool.clone());
        }
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|e| DenError::System(format!("failed to create bear sqlite data dir: {e}")))?;
        let db_path = self.data_dir.join(format!("{bear_id}.sqlite"));
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .pragma("journal_mode", "WAL")
            .pragma("synchronous", "NORMAL")
            .pragma("busy_timeout", "5000");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| DenError::System(format!("bear sqlite connect failed: {e}")))?;
        for statement in SCHEMA_SQL
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .map_err(|e| DenError::System(format!("bear sqlite schema failed: {e}")))?;
        }
        migrate_bear_sqlite_schema(&pool).await?;
        guard.insert(bear_id, pool.clone());
        Ok(pool)
    }
}
