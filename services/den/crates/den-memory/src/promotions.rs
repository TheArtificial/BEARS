use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

use super::{
    logical_path::LogicalMemoryPath,
    records::{head_record_for_logical_path, BearMemoryStore},
};

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
        r"
        INSERT INTO memory_promotions (
            promotion_id, bear_id, sequence_no, source_memory_id, target_memory_id,
            action, created_at, notes
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ",
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
    let logical = LogicalMemoryPath::shared_core(kind);
    promote_to_shared_core_at_path(
        store,
        source_memory_id,
        &logical.to_logical_path(),
        kind,
        content_text,
        author_profile,
        None,
    )
    .await
}

pub async fn promote_to_shared_core_at_path(
    store: &BearMemoryStore,
    source_memory_id: &str,
    target_path: &str,
    kind: &str,
    content_text: &str,
    author_profile: &str,
    salience: Option<&str>,
) -> Result<(String, String), DenError> {
    let logical = LogicalMemoryPath::from_logical_path(target_path);
    if logical.scope_type.as_str() != "shared" {
        return Err(DenError::ValidationError(
            "core promotion target_path must be under core/".to_string(),
        ));
    }
    let prior_head = head_record_for_logical_path(store, target_path).await?;
    if let Some(head) = &prior_head {
        if memory_claim_fingerprint(&head.content_text) == memory_claim_fingerprint(content_text) {
            let promotion_id = append_memory_promotion(
                store,
                source_memory_id,
                Some(&head.memory_id),
                "dedupe_core_noop",
                Some("Candidate matched existing core head; no new memory record written."),
            )
            .await?;
            return Ok((head.memory_id.clone(), promotion_id));
        }
    }
    let supersedes_memory_id = prior_head.map(|row| row.memory_id);
    let row = store
        .append_record_with_options(
            &logical,
            kind,
            author_profile,
            None,
            content_text,
            &serde_json::json!({
                "promoted_from": source_memory_id,
                "target_path": target_path,
                "promotion_policy": "reviewed_core_update",
            }),
            "normal",
            salience.unwrap_or("normal"),
            supersedes_memory_id.as_deref(),
        )
        .await?;
    // Provenance lives in `memory_promotions` (source → target); the legacy record→record
    // `memory_links` row was redundant and is retired with the entity relation layer (ADR-0042 §7).
    let notes = supersedes_memory_id
        .as_deref()
        .map(|id| format!("Supersedes prior core memory record {id}"));
    let promotion_id = append_memory_promotion(
        store,
        source_memory_id,
        Some(&row.memory_id),
        if supersedes_memory_id.is_some() {
            "supersede_core"
        } else {
            "promote_to_core"
        },
        notes.as_deref(),
    )
    .await?;
    Ok((row.memory_id, promotion_id))
}

pub fn memory_claim_fingerprint(text: &str) -> String {
    // ponytail: deterministic text fingerprint only; upgrade path is semantic dedup proposal mode.
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::new_test_store;

    #[tokio::test]
    async fn duplicate_promotion_records_noop_provenance_without_new_record() {
        let store = new_test_store().await;
        let (first_id, _) = promote_to_shared_core_at_path(
            &store,
            "proposal-1",
            "core/decisions/runtime.md",
            "runtime",
            "Same decision.",
            "curate",
            None,
        )
        .await
        .unwrap();

        let (second_id, promotion_id) = promote_to_shared_core_at_path(
            &store,
            "proposal-2",
            "core/decisions/runtime.md",
            "runtime",
            "Same   decision.\n",
            "curate",
            None,
        )
        .await
        .unwrap();

        assert_eq!(second_id, first_id);
        let records_at_path: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_records WHERE bear_id = ? AND logical_path = ?",
        )
        .bind(store.bear_id().to_string())
        .bind("core/decisions/runtime.md")
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(records_at_path, 1);
        let action: String = sqlx::query_scalar(
            "SELECT action FROM memory_promotions WHERE bear_id = ? AND promotion_id = ?",
        )
        .bind(store.bear_id().to_string())
        .bind(promotion_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(action, "dedupe_core_noop");
    }

    #[tokio::test]
    async fn duplicate_promotion_ignores_case_and_punctuation_noise() {
        let store = new_test_store().await;
        let (first_id, _) = promote_to_shared_core_at_path(
            &store,
            "proposal-1",
            "core/decisions/runtime.md",
            "runtime",
            "Use BearWire for armature transport.",
            "curate",
            None,
        )
        .await
        .unwrap();

        let (second_id, _) = promote_to_shared_core_at_path(
            &store,
            "proposal-2",
            "core/decisions/runtime.md",
            "runtime",
            "use bearwire for armature transport",
            "curate",
            None,
        )
        .await
        .unwrap();

        assert_eq!(second_id, first_id);
        let records_at_path: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_records WHERE bear_id = ? AND logical_path = ?",
        )
        .bind(store.bear_id().to_string())
        .bind("core/decisions/runtime.md")
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(records_at_path, 1);
    }

    #[tokio::test]
    async fn promotion_at_path_supersedes_previous_core_head() {
        let store = new_test_store().await;
        let (first_id, _) = promote_to_shared_core_at_path(
            &store,
            "proposal-1",
            "core/decisions/runtime.md",
            "runtime",
            "First decision.",
            "curate",
            None,
        )
        .await
        .unwrap();

        let (second_id, _) = promote_to_shared_core_at_path(
            &store,
            "proposal-2",
            "core/decisions/runtime.md",
            "runtime",
            "Updated decision.",
            "curate",
            None,
        )
        .await
        .unwrap();

        let first_invalid_at: Option<String> = sqlx::query_scalar(
            "SELECT invalid_at FROM memory_records WHERE bear_id = ? AND memory_id = ?",
        )
        .bind(store.bear_id().to_string())
        .bind(&first_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert!(first_invalid_at.is_some());

        let head = head_record_for_logical_path(&store, "core/decisions/runtime.md")
            .await
            .unwrap()
            .expect("head record");
        assert_eq!(head.memory_id, second_id);
        assert_eq!(
            head.supersedes_memory_id.as_deref(),
            Some(first_id.as_str())
        );
    }
}
