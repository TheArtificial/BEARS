use time::OffsetDateTime;
use uuid::Uuid;

use crate::errors::CustomError;

use super::{logical_path::LogicalMemoryPath, records::BearMemoryStore};

pub async fn append_memory_promotion(
    store: &BearMemoryStore,
    source_memory_id: &str,
    target_memory_id: Option<&str>,
    action: &str,
    notes: Option<&str>,
) -> Result<String, CustomError> {
    let promotion_id = Uuid::new_v4().to_string();
    let sequence_no = store.next_sequence().await?;
    let created_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| CustomError::System(format!("timestamp format failed: {e}")))?;
    sqlx::query(
        r#"
        INSERT INTO memory_promotions (
            promotion_id, bear_id, sequence_no, source_memory_id, target_memory_id,
            action, created_at, notes
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&promotion_id)
    .bind(store.bear_id().to_string())
    .bind(sequence_no)
    .bind(source_memory_id)
    .bind(target_memory_id)
    .bind(action)
    .bind(&created_at)
    .bind(notes)
    .execute(store.pool())
    .await
    .map_err(|e| CustomError::System(format!("sqlite append promotion failed: {e}")))?;
    Ok(promotion_id)
}

pub async fn promote_to_shared_core(
    store: &BearMemoryStore,
    source_memory_id: &str,
    kind: &str,
    content_text: &str,
    author_role: &str,
) -> Result<(String, String), CustomError> {
    use super::records::append_memory_record;
    let logical = LogicalMemoryPath::shared_core(kind);
    let row = append_memory_record(
        store,
        &logical,
        kind,
        author_role,
        None,
        content_text,
        &serde_json::json!({ "promoted_from": source_memory_id }),
    )
    .await?;
    let promotion_id = append_memory_promotion(
        store,
        source_memory_id,
        Some(&row.memory_id),
        "promote_to_core",
        None,
    )
    .await?;
    Ok((row.memory_id, promotion_id))
}
