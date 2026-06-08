use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    config::Config,
    core::memory::store::{
        append_memory_record, list_records_for_logical_path, LogicalMemoryPath, MemoryStoreManager,
    },
    errors::CustomError,
};

use super::store::BearMemoryStore;

pub async fn sqlite_write_role_entry(
    stores: &MemoryStoreManager,
    config: &Config,
    bear_id: Uuid,
    role: &str,
    kind: &str,
    title: &str,
    body: &str,
    tags: &[String],
    source: Option<Value>,
    author: Option<String>,
) -> Result<Value, CustomError> {
    let store = stores.store_for_bear(bear_id).await?;
    let logical = LogicalMemoryPath::role_local(role, kind);
    let content = format!("# {title}\n\n{body}");
    let metadata = json!({
        "title": title,
        "tags": tags,
        "source": source,
        "author": author,
        "storage": "sqlite",
        "runtime": config.agent_runtime_mode.as_str(),
    });
    let row = append_memory_record(
        &store,
        &logical,
        kind,
        role,
        None,
        &content,
        &metadata,
    )
    .await?;
    Ok(json!({
        "bear_id": bear_id,
        "role": role,
        "kind": row.kind,
        "entry_id": row.memory_id,
        "path": row.logical_path,
        "sequence_no": row.sequence_no,
        "storage": "sqlite",
    }))
}

pub async fn sqlite_memory_browse(
    store: &BearMemoryStore,
    role: &str,
) -> Result<Value, CustomError> {
    let rows = sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT logical_path
        FROM memory_records
        WHERE bear_id = ? AND scope_role = ? AND logical_path IS NOT NULL
        ORDER BY logical_path ASC
        "#,
    )
    .bind(store.bear_id().to_string())
    .bind(role)
    .fetch_all(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("sqlite memory browse failed: {e}")))?;
    let children: Vec<Value> = rows
        .into_iter()
        .map(|path| {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            json!({
                "name": name,
                "path": path,
                "type": "file",
            })
        })
        .collect();
    Ok(json!({
        "ok": true,
        "configured": true,
        "storage": "sqlite",
        "role": role,
        "children": children,
    }))
}

pub async fn sqlite_memory_read(
    store: &BearMemoryStore,
    logical_path: &str,
) -> Result<Value, CustomError> {
    let rows = list_records_for_logical_path(store, logical_path, 20).await?;
    if rows.is_empty() {
        return Ok(json!({
            "ok": false,
            "configured": true,
            "storage": "sqlite",
            "path": logical_path,
            "message": "no records at path",
        }));
    }
    let body: String = rows
        .iter()
        .rev()
        .map(|r| r.content_text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    Ok(json!({
        "ok": true,
        "configured": true,
        "storage": "sqlite",
        "path": logical_path,
        "content": body,
        "record_count": rows.len(),
        "latest_sequence_no": rows.first().map(|r| r.sequence_no),
    }))
}

pub async fn sqlite_memory_search(
    store: &BearMemoryStore,
    role: &str,
    query: &str,
    limit: i64,
) -> Result<Value, CustomError> {
    let pattern = format!("%{query}%");
    let rows = sqlx::query_as::<_, (String, String, String, i64)>(
        r#"
        SELECT memory_id, logical_path, content_text, sequence_no
        FROM memory_records
        WHERE bear_id = ? AND scope_role = ? AND content_text LIKE ?
        ORDER BY sequence_no DESC
        LIMIT ?
        "#,
    )
    .bind(store.bear_id().to_string())
    .bind(role)
    .bind(pattern)
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("sqlite memory search failed: {e}")))?;
    let hits: Vec<Value> = rows
        .into_iter()
        .map(|(memory_id, path, content, sequence_no)| {
            json!({
                "memory_id": memory_id,
                "path": path,
                "snippet": content.chars().take(240).collect::<String>(),
                "sequence_no": sequence_no,
            })
        })
        .collect();
    Ok(json!({
        "ok": true,
        "configured": true,
        "storage": "sqlite",
        "query": query,
        "hits": hits,
    }))
}

pub async fn sqlite_memory_status(
    store: &BearMemoryStore,
    role: &str,
) -> Result<Value, CustomError> {
    let file_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(DISTINCT logical_path)
        FROM memory_records
        WHERE bear_id = ? AND scope_role = ?
        "#,
    )
    .bind(store.bear_id().to_string())
    .bind(role)
    .fetch_one(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("sqlite memory status failed: {e}")))?;
    Ok(json!({
        "configured": true,
        "available": true,
        "storage": "sqlite",
        "role": role,
        "file_count": file_count,
    }))
}
