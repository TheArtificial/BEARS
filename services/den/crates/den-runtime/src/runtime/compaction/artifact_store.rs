use den_core::DenError;
use serde_json::to_value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::runtime_conversations::{RuntimeCompactionTriggerKind, RuntimeIterativeSummary};

use super::{RuntimeCompactionDecision, RuntimeCompactionPolicy};

#[derive(Debug, Clone)]
pub struct CompactionArtifactRecord {
    pub artifact_id: Uuid,
    pub summary: RuntimeIterativeSummary,
    pub source_message_start_seq: i64,
    pub source_message_end_seq: i64,
    pub policy_version: String,
}

pub async fn load_latest_iterative_summary(
    pool: &PgPool,
    bear_id: Uuid,
    external_conversation_id: &str,
) -> Result<Option<CompactionArtifactRecord>, DenError> {
    let Some(conversation_uuid) =
        resolve_conversation_uuid(pool, bear_id, external_conversation_id).await?
    else {
        return Ok(None);
    };

    let row = sqlx::query_as::<_, (Uuid, serde_json::Value, i64, i64, String)>(
        r"
        SELECT id, artifact_json, source_message_start_seq, source_message_end_seq, policy_version
        FROM conversation_compaction_artifacts
        WHERE conversation_id = $1
          AND artifact_kind = 'iterative_summary'
          AND superseded_by IS NULL
        ORDER BY created_at DESC
        LIMIT 1
        ",
    )
    .bind(conversation_uuid)
    .fetch_optional(pool)
    .await
    .map_err(|err| DenError::Database(format!("load compaction artifact: {err}")))?;

    let Some((artifact_id, artifact_json, start_seq, end_seq, policy_version)) = row else {
        return Ok(None);
    };

    let summary: RuntimeIterativeSummary = serde_json::from_value(artifact_json)
        .map_err(|err| DenError::Database(format!("decode compaction artifact_json: {err}")))?;

    Ok(Some(CompactionArtifactRecord {
        artifact_id,
        summary,
        source_message_start_seq: start_seq,
        source_message_end_seq: end_seq,
        policy_version,
    }))
}

pub async fn insert_iterative_summary_artifact(
    pool: &PgPool,
    bear_id: Uuid,
    external_conversation_id: &str,
    decision: &RuntimeCompactionDecision,
    policy: &RuntimeCompactionPolicy,
    trigger: RuntimeCompactionTriggerKind,
    source_message_start_seq: i64,
    source_message_end_seq: i64,
    summary: &RuntimeIterativeSummary,
) -> Result<Uuid, DenError> {
    let conversation_uuid = resolve_conversation_uuid(pool, bear_id, external_conversation_id)
        .await?
        .ok_or_else(|| {
            DenError::NotFound(format!(
                "conversation not found for compaction artifact: {external_conversation_id}"
            ))
        })?;

    let artifact_json = to_value(summary)
        .map_err(|err| DenError::System(format!("serialize compaction summary: {err}")))?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| DenError::Database(format!("begin compaction artifact tx: {err}")))?;

    let prior_id = sqlx::query_scalar::<_, Option<Uuid>>(
        r"
        SELECT id
        FROM conversation_compaction_artifacts
        WHERE conversation_id = $1
          AND artifact_kind = 'iterative_summary'
          AND superseded_by IS NULL
        ORDER BY created_at DESC
        LIMIT 1
        ",
    )
    .bind(conversation_uuid)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| DenError::Database(format!("select prior compaction artifact: {err}")))?
    .flatten();

    let inserted_id = sqlx::query_scalar::<_, Uuid>(
        r"
        INSERT INTO conversation_compaction_artifacts (
            conversation_id,
            artifact_kind,
            policy_version,
            trigger,
            source_message_start_seq,
            source_message_end_seq,
            source_group_start,
            source_group_end,
            artifact_json
        )
        VALUES ($1, 'iterative_summary', $2, $3, $4, $5, $6, $7, $8)
        RETURNING id
        ",
    )
    .bind(conversation_uuid)
    .bind(&policy.policy_version)
    .bind(trigger.as_str())
    .bind(source_message_start_seq)
    .bind(source_message_end_seq)
    .bind(decision.selected_group_start as i32)
    .bind(decision.selected_group_end as i32)
    .bind(artifact_json)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| DenError::Database(format!("insert compaction artifact: {err}")))?;

    if let Some(prior_id) = prior_id {
        sqlx::query(
            r"
            UPDATE conversation_compaction_artifacts
            SET superseded_by = $1
            WHERE id = $2
            ",
        )
        .bind(inserted_id)
        .bind(prior_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| DenError::Database(format!("supersede prior compaction artifact: {err}")))?;
    }

    tx.commit()
        .await
        .map_err(|err| DenError::Database(format!("commit compaction artifact tx: {err}")))?;

    Ok(inserted_id)
}

async fn resolve_conversation_uuid(
    pool: &PgPool,
    bear_id: Uuid,
    external_conversation_id: &str,
) -> Result<Option<Uuid>, DenError> {
    let row = sqlx::query_scalar::<_, Uuid>(
        r"
        SELECT id
        FROM conversations
        WHERE bear_id = $1 AND external_conversation_id = $2
        LIMIT 1
        ",
    )
    .bind(bear_id)
    .bind(external_conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| DenError::Database(format!("resolve conversation uuid: {err}")))?;
    Ok(row)
}
