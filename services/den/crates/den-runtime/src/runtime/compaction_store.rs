use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
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

pub async fn record_runtime_compaction_event(
    pool: &PgPool,
    event: &RuntimeCompactionEvent,
) -> Result<(), DenError> {
    let event_hash = runtime_compaction_event_hash(event)?;
    let boundary = serde_json::to_value(&event.boundary).map_err(|err| {
        DenError::System(format!("serialize compaction boundary: {err}"))
    })?;
    let artifact = serde_json::to_value(&event.artifact).map_err(|err| {
        DenError::System(format!("serialize compaction artifact: {err}"))
    })?;
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
    .bind(format!("{:?}", event.trigger))
    .bind(&event.policy_version)
    .bind(format!("{:?}", event.status))
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
    let rows = sqlx::query(
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

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let artifact = row
            .try_get::<Option<serde_json::Value>, _>("artifact")
            .map_err(|err| DenError::Database(format!("decode compaction artifact: {err}")))?;
        items.push(CompactionStatusResponse {
            status: row
                .try_get::<String, _>("status")
                .map_err(|err| DenError::Database(format!("decode compaction status: {err}")))?,
            policy_version: row.try_get::<String, _>("policy_version").map_err(|err| {
                DenError::Database(format!("decode compaction policy_version: {err}"))
            })?,
            trigger: Some(
                row.try_get::<String, _>("trigger")
                    .map_err(|err| DenError::Database(format!("decode compaction trigger: {err}")))?,
            ),
            created_at: Some(
                row.try_get::<String, _>("created_at")
                    .map_err(|err| DenError::Database(format!("decode compaction created_at: {err}")))?,
            ),
            source_group_start: row
                .try_get::<Option<i32>, _>("source_group_start")
                .map_err(|err| {
                    DenError::Database(format!("decode compaction source_group_start: {err}"))
                })?
                .map(|v| v as usize),
            source_group_end: row
                .try_get::<Option<i32>, _>("source_group_end")
                .map_err(|err| {
                    DenError::Database(format!("decode compaction source_group_end: {err}"))
                })?
                .map(|v| v as usize),
            diagnostic: row
                .try_get::<Option<String>, _>("diagnostic")
                .map_err(|err| {
                    DenError::Database(format!("decode compaction diagnostic: {err}"))
                })?,
            artifact,
            context_envelope: None,
            prompt_memory_diagnostic: None,
            latest_artifact: None,
        });
    }
    Ok(items)
}

pub async fn latest_compaction_artifact_for_conversation(
    pool: &PgPool,
    conversation_uuid: Uuid,
) -> Result<Option<CompactionArtifactResponse>, DenError> {
    let row = sqlx::query(
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

    let Some(row) = row else {
        return Ok(None);
    };

    Ok(Some(CompactionArtifactResponse {
        id: row
            .try_get::<Uuid, _>("id")
            .map_err(|err| DenError::Database(format!("decode compaction artifact id: {err}")))?,
        artifact_kind: row.try_get::<String, _>("artifact_kind").map_err(|err| {
            DenError::Database(format!("decode compaction artifact_kind: {err}"))
        })?,
        policy_version: row.try_get::<String, _>("policy_version").map_err(|err| {
            DenError::Database(format!("decode compaction artifact policy_version: {err}"))
        })?,
        trigger: row.try_get::<String, _>("trigger").map_err(|err| {
            DenError::Database(format!("decode compaction artifact trigger: {err}"))
        })?,
        source_message_start_seq: row.try_get::<i64, _>("source_message_start_seq").map_err(
            |err| DenError::Database(format!("decode compaction source_message_start_seq: {err}")),
        )?,
        source_message_end_seq: row.try_get::<i64, _>("source_message_end_seq").map_err(
            |err| DenError::Database(format!("decode compaction source_message_end_seq: {err}")),
        )?,
        source_group_start: row
            .try_get::<Option<i32>, _>("source_group_start")
            .map_err(|err| DenError::Database(format!("decode compaction source_group_start: {err}")))?
            .map(|v| v as usize),
        source_group_end: row
            .try_get::<Option<i32>, _>("source_group_end")
            .map_err(|err| DenError::Database(format!("decode compaction source_group_end: {err}")))?
            .map(|v| v as usize),
        artifact_json: row.try_get::<serde_json::Value, _>("artifact_json").map_err(|err| {
            DenError::Database(format!("decode compaction artifact_json: {err}"))
        })?,
        superseded_by: row.try_get::<Option<Uuid>, _>("superseded_by").map_err(|err| {
            DenError::Database(format!("decode compaction superseded_by: {err}"))
        })?,
        created_at: row
            .try_get::<String, _>("created_at")
            .map_err(|err| DenError::Database(format!("decode compaction artifact created_at: {err}")))?
    }))
}

fn runtime_compaction_event_hash(event: &RuntimeCompactionEvent) -> Result<String, DenError> {
    let payload = serde_json::json!({
        "conversation_id": event.conversation_id,
        "trigger": format!("{:?}", event.trigger),
        "policy_version": event.policy_version,
        "status": format!("{:?}", event.status),
        "boundary": event.boundary,
        "source_group_start": event.source_group_start,
        "source_group_end": event.source_group_end,
        "artifact": event.artifact,
        "diagnostic": event.diagnostic,
    });
    let bytes = serde_json::to_vec(&payload)
        .map_err(|err| DenError::System(format!("serialize compaction event hash payload: {err}")))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{:x}", digest))
}
