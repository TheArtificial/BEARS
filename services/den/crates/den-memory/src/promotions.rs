use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

use super::{logical_path::LogicalMemoryPath, records::BearMemoryStore};

pub async fn append_memory_promotion(
    store: &BearMemoryStore,
    source_memory_id: &str,
    target_memory_id: Option<&str>,
    action: &str,
    notes: Option<&str>,
) -> Result<String, DenError> {
    let promotion_id = Uuid::new_v4().to_string();
    let sequence_no = store.next_sequence().await?;
    let created_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| DenError::System(format!("timestamp format failed: {e}")))?;
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
    .map_err(|e| DenError::System(format!("sqlite append promotion failed: {e}")))?;
    Ok(promotion_id)
}

pub async fn promote_to_shared_core(
    store: &BearMemoryStore,
    source_memory_id: &str,
    kind: &str,
    content_text: &str,
    author_profile: &str,
) -> Result<(String, String), DenError> {
    use super::records::append_memory_record;
    let logical = LogicalMemoryPath::shared_core(kind);
    let row = append_memory_record(
        store,
        &logical,
        kind,
        author_profile,
        None,
        content_text,
        &serde_json::json!({ "promoted_from": source_memory_id }),
    )
    .await?;
    // Provenance lives in `memory_promotions` (source → target); the legacy record→record
    // `memory_links` row was redundant and is retired with the entity relation layer (ADR-0042 §7).
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
