use sqlx::SqlitePool;

use crate::errors::CustomError;

/// Upgrade per-Bear SQLite files created before profile vocabulary cleanup (ADR-0036).
pub async fn migrate_bear_sqlite_schema(pool: &SqlitePool) -> Result<(), CustomError> {
    let columns = sqlx::query_scalar::<_, String>(
        "SELECT name FROM pragma_table_info('memory_records')",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| CustomError::System(format!("bear sqlite pragma failed: {e}")))?;
    let names = columns;

    if names.iter().any(|c| c == "scope_role") && !names.iter().any(|c| c == "scope_profile") {
        sqlx::query("ALTER TABLE memory_records RENAME COLUMN scope_role TO scope_profile")
            .execute(pool)
            .await
            .map_err(|e| CustomError::System(format!("rename scope_role failed: {e}")))?;
    }
    if names.iter().any(|c| c == "author_role") && !names.iter().any(|c| c == "author_profile") {
        sqlx::query("ALTER TABLE memory_records RENAME COLUMN author_role TO author_profile")
            .execute(pool)
            .await
            .map_err(|e| CustomError::System(format!("rename author_role failed: {e}")))?;
    }

    let table_sql: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memory_records'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| CustomError::System(format!("bear sqlite table info failed: {e}")))?;

    let needs_scope_vocab_rebuild = table_sql
        .as_deref()
        .map(|sql| sql.contains("'role_local'"))
        .unwrap_or(false);

    if needs_scope_vocab_rebuild {
        rebuild_memory_records_scope_vocab(pool).await?;
        return Ok(());
    }

    sqlx::query(
        "UPDATE memory_records SET scope_type = 'profile_local' WHERE scope_type = 'role_local'",
    )
    .execute(pool)
    .await
    .map_err(|e| CustomError::System(format!("migrate scope_type values failed: {e}")))?;

    Ok(())
}

async fn rebuild_memory_records_scope_vocab(pool: &SqlitePool) -> Result<(), CustomError> {
    sqlx::query("BEGIN IMMEDIATE")
        .execute(pool)
        .await
        .map_err(|e| CustomError::System(format!("bear sqlite migration begin failed: {e}")))?;

    let migration = async {
        sqlx::query(
            r#"
            CREATE TABLE memory_records_new (
                memory_id TEXT PRIMARY KEY,
                bear_id TEXT NOT NULL,
                sequence_no INTEGER NOT NULL,
                scope_type TEXT NOT NULL CHECK (scope_type IN ('profile_local', 'shared')),
                scope_profile TEXT NULL,
                kind TEXT NOT NULL,
                entity_ref TEXT NULL,
                author_profile TEXT NOT NULL,
                author_agent_id TEXT NULL,
                created_at TEXT NOT NULL,
                content_text TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT '{}',
                supersedes_memory_id TEXT NULL,
                visibility TEXT NOT NULL DEFAULT 'normal',
                logical_path TEXT NULL,
                work_surface_ref TEXT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| CustomError::System(format!("create memory_records_new failed: {e}")))?;

        sqlx::query(
            r#"
            INSERT INTO memory_records_new (
                memory_id, bear_id, sequence_no, scope_type, scope_profile, kind, entity_ref,
                author_profile, author_agent_id, created_at, content_text, metadata_json,
                supersedes_memory_id, visibility, logical_path, work_surface_ref
            )
            SELECT
                memory_id,
                bear_id,
                sequence_no,
                CASE scope_type WHEN 'role_local' THEN 'profile_local' ELSE scope_type END,
                scope_profile,
                kind,
                entity_ref,
                author_profile,
                author_agent_id,
                created_at,
                content_text,
                metadata_json,
                supersedes_memory_id,
                visibility,
                logical_path,
                work_surface_ref
            FROM memory_records
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| CustomError::System(format!("copy memory_records scope vocab failed: {e}")))?;

        sqlx::query("DROP TABLE memory_records")
            .execute(pool)
            .await
            .map_err(|e| CustomError::System(format!("drop legacy memory_records failed: {e}")))?;

        sqlx::query("ALTER TABLE memory_records_new RENAME TO memory_records")
            .execute(pool)
            .await
            .map_err(|e| CustomError::System(format!("rename memory_records_new failed: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_memory_records_bear_sequence ON memory_records (bear_id, sequence_no)",
        )
        .execute(pool)
        .await
        .map_err(|e| CustomError::System(format!("recreate memory_records index failed: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_memory_records_logical_path ON memory_records (bear_id, logical_path)",
        )
        .execute(pool)
        .await
        .map_err(|e| CustomError::System(format!("recreate memory_records logical_path index failed: {e}")))?;

        Ok::<(), CustomError>(())
    }
    .await;

    match migration {
        Ok(()) => {
            sqlx::query("COMMIT")
                .execute(pool)
                .await
                .map_err(|e| CustomError::System(format!("bear sqlite migration commit failed: {e}")))?;
        }
        Err(err) => {
            let _ = sqlx::query("ROLLBACK").execute(pool).await;
            return Err(err);
        }
    }

    Ok(())
}
