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

    sqlx::query(
        "UPDATE memory_records SET scope_type = 'profile_local' WHERE scope_type = 'role_local'",
    )
    .execute(pool)
    .await
    .map_err(|e| CustomError::System(format!("migrate scope_type values failed: {e}")))?;

    Ok(())
}
