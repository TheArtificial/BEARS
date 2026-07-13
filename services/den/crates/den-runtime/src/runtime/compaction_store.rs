use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::runtime_compaction_observability::RuntimeCompactionEvent;
use den_core::DenError;
use serde::Serialize;

/// Serialized compaction status for a conversation, as surfaced to client/web clients.
///
/// Produced here by the runtime compaction store and consumed by the `den` API edge
/// (re-exported as `crate::core::runtime_compaction_store::CompactionStatusResponse`).
#[derive(Debug, Clone, Serialize)]
pub struct CompactionStatusResponse {
    pub status: String,
    pub policy_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_group_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_group_end: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_envelope: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_memory_diagnostic: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_artifact: Option<CompactionArtifactResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactionArtifactResponse {
    pub id: Uuid,
    pub artifact_kind: String,
    pub policy_version: String,
    pub trigger: String,
    pub source_message_start_seq: i64,
    pub source_message_end_seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_group_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_group_end: Option<usize>,
    pub artifact_json: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<Uuid>,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct CompactionStatusRow {
    status: String,
    policy_version: String,
    trigger: String,
    source_group_start: Option<i32>,
    source_group_end: Option<i32>,
    diagnostic: Option<String>,
    artifact: Option<serde_json::Value>,
    created_at: String,
}

impl From<CompactionStatusRow> for CompactionStatusResponse {
    fn from(row: CompactionStatusRow) -> Self {
        Self {
            status: row.status,
            policy_version: row.policy_version,
            trigger: Some(row.trigger),
            created_at: Some(row.created_at),
            source_group_start: row.source_group_start.map(|value| value as usize),
            source_group_end: row.source_group_end.map(|value| value as usize),
            diagnostic: row.diagnostic,
            artifact: row.artifact,
            context_envelope: None,
            prompt_memory_diagnostic: None,
            latest_artifact: None,
        }
    }
}

#[derive(sqlx::FromRow)]
struct CompactionArtifactRow {
    id: Uuid,
    artifact_kind: String,
    policy_version: String,
    trigger: String,
    source_message_start_seq: i64,
    source_message_end_seq: i64,
    source_group_start: Option<i32>,
    source_group_end: Option<i32>,
    artifact_json: serde_json::Value,
    superseded_by: Option<Uuid>,
    created_at: String,
}

impl From<CompactionArtifactRow> for CompactionArtifactResponse {
    fn from(row: CompactionArtifactRow) -> Self {
        Self {
            id: row.id,
            artifact_kind: row.artifact_kind,
            policy_version: row.policy_version,
            trigger: row.trigger,
            source_message_start_seq: row.source_message_start_seq,
            source_message_end_seq: row.source_message_end_seq,
            source_group_start: row.source_group_start.map(|value| value as usize),
            source_group_end: row.source_group_end.map(|value| value as usize),
            artifact_json: row.artifact_json,
            superseded_by: row.superseded_by,
            created_at: row.created_at,
        }
    }
}

pub async fn record_runtime_compaction_event(
    pool: &PgPool,
    event: &RuntimeCompactionEvent,
) -> Result<(), DenError> {
    let event_hash = runtime_compaction_event_hash(event)?;
    let boundary = runtime_compaction_boundary_json(event)?;
    let artifact = event
        .artifact
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|err| DenError::System(format!("serialize compaction artifact: {err}")))?;
    sqlx::query(
        r"
        INSERT INTO runtime_compaction_events (
            conversation_id,
            trigger,
            policy_version,
            status,
            event_hash,
            boundary,
            source_group_start,
            source_group_end,
            artifact,
            diagnostic
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (conversation_id, event_hash) DO NOTHING
        ",
    )
    .bind(&event.conversation_id)
    .bind(event.trigger.as_str())
    .bind(&event.policy_version)
    .bind(event.status.as_str())
    .bind(&event_hash)
    .bind(boundary)
    .bind(event.source_group_start.map(|v| v as i32))
    .bind(event.source_group_end.map(|v| v as i32))
    .bind(artifact)
    .bind(&event.diagnostic)
    .execute(pool)
    .await
    .map_err(|err| DenError::Database(format!("insert runtime_compaction_events: {err}")))?;
    Ok(())
}

pub async fn list_runtime_compaction_events(
    pool: &PgPool,
    conversation_id: &str,
    limit: i64,
) -> Result<Vec<CompactionStatusResponse>, DenError> {
    let rows = sqlx::query_as::<_, CompactionStatusRow>(
        r#"
        SELECT
            trigger,
            status,
            policy_version,
            source_group_start,
            source_group_end,
            diagnostic,
            artifact,
            to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at
        FROM runtime_compaction_events
        WHERE conversation_id = $1
        ORDER BY created_at DESC
        LIMIT $2
        "#,
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| DenError::Database(format!("select runtime_compaction_events: {err}")))?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn latest_compaction_artifact_for_conversation(
    pool: &PgPool,
    conversation_uuid: Uuid,
) -> Result<Option<CompactionArtifactResponse>, DenError> {
    let row = sqlx::query_as::<_, CompactionArtifactRow>(
        r#"
        SELECT
            id,
            artifact_kind,
            policy_version,
            trigger,
            source_message_start_seq,
            source_message_end_seq,
            source_group_start,
            source_group_end,
            artifact_json,
            superseded_by,
            to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at
        FROM conversation_compaction_artifacts
        WHERE conversation_id = $1
          AND superseded_by IS NULL
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(conversation_uuid)
    .fetch_optional(pool)
    .await
    .map_err(|err| DenError::Database(format!("select latest compaction artifact: {err}")))?;

    Ok(row.map(Into::into))
}

fn runtime_compaction_boundary_json(
    event: &RuntimeCompactionEvent,
) -> Result<Option<serde_json::Value>, DenError> {
    event
        .boundary
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|err| DenError::System(format!("serialize compaction boundary: {err}")))
}

fn runtime_compaction_event_hash(event: &RuntimeCompactionEvent) -> Result<String, DenError> {
    let payload = serde_json::json!({
        "conversation_id": event.conversation_id,
        "trigger": event.trigger.as_str(),
        "policy_version": event.policy_version,
        "status": event.status.as_str(),
        "boundary": event.boundary,
        "source_group_start": event.source_group_start,
        "source_group_end": event.source_group_end,
        "artifact": event.artifact,
        "diagnostic": event.diagnostic,
    });
    let bytes = serde_json::to_vec(&payload).map_err(|err| {
        DenError::System(format!("serialize compaction event hash payload: {err}"))
    })?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{:x}", digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        runtime_compaction_observability::RuntimeCompactionEventStatus,
        runtime_conversations::{RuntimeCompactionBoundary, RuntimeCompactionTriggerKind},
    };

    fn event_with_boundary(boundary: Option<RuntimeCompactionBoundary>) -> RuntimeCompactionEvent {
        RuntimeCompactionEvent {
            conversation_id: "conv-1".to_string(),
            trigger: RuntimeCompactionTriggerKind::Manual,
            policy_version: "policy-v1".to_string(),
            status: RuntimeCompactionEventStatus::Skipped,
            boundary,
            source_group_start: None,
            source_group_end: None,
            artifact: None,
            diagnostic: Some("below threshold".to_string()),
        }
    }

    #[test]
    fn missing_compaction_boundary_binds_as_sql_null_not_json_null() {
        let boundary = runtime_compaction_boundary_json(&event_with_boundary(None)).unwrap();
        assert!(boundary.is_none());
    }

    #[test]
    fn present_compaction_boundary_binds_as_json_object() {
        let event = event_with_boundary(Some(RuntimeCompactionBoundary {
            retained_group_count: 2,
            compacted_group_count: 3,
        }));
        let boundary = runtime_compaction_boundary_json(&event).unwrap().unwrap();
        assert_eq!(
            boundary
                .get("retained_group_count")
                .and_then(|v| v.as_u64()),
            Some(2)
        );
        assert_eq!(
            boundary
                .get("compacted_group_count")
                .and_then(|v| v.as_u64()),
            Some(3)
        );
    }
}
