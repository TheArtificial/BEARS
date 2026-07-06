use den_core::DenError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{RuntimeCheckpointRequest, RuntimeCheckpointResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointValidationStatus {
    Requested,
    Valid,
    Invalid,
    Superseded,
}

impl CheckpointValidationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointVisibility {
    AuditOnly,
    LiveEphemeral,
    ModelVisibleHidden,
}

impl CheckpointVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuditOnly => "audit_only",
            Self::LiveEphemeral => "live_ephemeral",
            Self::ModelVisibleHidden => "model_visible_hidden",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReplayPolicy {
    None,
    SummaryOnce,
    UntilSuperseded,
}

impl CheckpointReplayPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SummaryOnce => "summary_once",
            Self::UntilSuperseded => "until_superseded",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CheckpointArtifactInput {
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub request: RuntimeCheckpointRequest,
    pub visibility: CheckpointVisibility,
    pub replay_policy: CheckpointReplayPolicy,
}

#[derive(Debug, Clone)]
pub struct CheckpointResponseInput {
    pub run_id: String,
    pub checkpoint_id: String,
    pub response: RuntimeCheckpointResponse,
    pub validation_status: CheckpointValidationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointArtifactRow {
    pub id: Uuid,
    pub run_id: String,
    pub turn_step_id: Option<Uuid>,
    pub checkpoint_id: String,
    pub reason: String,
    pub control_level: String,
    pub request: Value,
    pub response: Option<Value>,
    pub validation_status: String,
    pub visibility: String,
    pub replay_policy: String,
    pub related_task_list_id: Option<String>,
    pub related_task_item_id: Option<String>,
    pub related_docket_task_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub async fn record_checkpoint_request(
    pool: &PgPool,
    input: CheckpointArtifactInput,
) -> Result<CheckpointArtifactRow, DenError> {
    let request_json = serde_json::to_value(&input.request)
        .map_err(|err| DenError::System(format!("serialize checkpoint request: {err}")))?;
    let task_context = input.request.task_context.as_ref();
    let related_docket_task_id = task_context
        .and_then(|context| context.docket_task_id.as_deref())
        .and_then(|value| Uuid::parse_str(value).ok());

    let row = sqlx::query(
        r#"
        INSERT INTO bear_run_checkpoints (
            run_id, turn_step_id, checkpoint_id, reason, control_level, request,
            validation_status, visibility, replay_policy, related_task_list_id,
            related_task_item_id, related_docket_task_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, 'requested', $7, $8, $9, $10, $11)
        ON CONFLICT (run_id, checkpoint_id) DO UPDATE SET
            request = EXCLUDED.request,
            reason = EXCLUDED.reason,
            control_level = EXCLUDED.control_level,
            validation_status = 'requested',
            visibility = EXCLUDED.visibility,
            replay_policy = EXCLUDED.replay_policy,
            related_task_list_id = EXCLUDED.related_task_list_id,
            related_task_item_id = EXCLUDED.related_task_item_id,
            related_docket_task_id = EXCLUDED.related_docket_task_id,
            response = NULL,
            updated_at = NOW()
        RETURNING
            id, run_id, turn_step_id, checkpoint_id, reason, control_level, request,
            response, validation_status, visibility, replay_policy, related_task_list_id,
            related_task_item_id, related_docket_task_id, created_at, updated_at
        "#,
    )
    .bind(&input.run_id)
    .bind(input.turn_step_id)
    .bind(&input.request.checkpoint_id)
    .bind(input.request.reason.as_str())
    .bind(input.request.control_level.as_str())
    .bind(request_json)
    .bind(input.visibility.as_str())
    .bind(input.replay_policy.as_str())
    .bind(task_context.and_then(|context| context.task_list_id.as_deref()))
    .bind(task_context.and_then(|context| context.active_item_id.as_deref()))
    .bind(related_docket_task_id)
    .fetch_one(pool)
    .await?;

    Ok(row_to_checkpoint(row))
}

pub async fn record_checkpoint_response(
    pool: &PgPool,
    input: CheckpointResponseInput,
) -> Result<CheckpointArtifactRow, DenError> {
    let response_json = serde_json::to_value(&input.response)
        .map_err(|err| DenError::System(format!("serialize checkpoint response: {err}")))?;
    let row = sqlx::query(
        r#"
        UPDATE bear_run_checkpoints
        SET response = $3,
            validation_status = $4,
            updated_at = NOW()
        WHERE run_id = $1 AND checkpoint_id = $2
        RETURNING
            id, run_id, turn_step_id, checkpoint_id, reason, control_level, request,
            response, validation_status, visibility, replay_policy, related_task_list_id,
            related_task_item_id, related_docket_task_id, created_at, updated_at
        "#,
    )
    .bind(&input.run_id)
    .bind(&input.checkpoint_id)
    .bind(response_json)
    .bind(input.validation_status.as_str())
    .fetch_optional(pool)
    .await?;

    row.map(row_to_checkpoint).ok_or_else(|| {
        DenError::NotFound(format!(
            "checkpoint artifact not found: run_id={} checkpoint_id={}",
            input.run_id, input.checkpoint_id
        ))
    })
}

pub async fn list_checkpoints_for_run(
    pool: &PgPool,
    run_id: &str,
) -> Result<Vec<CheckpointArtifactRow>, DenError> {
    let rows = sqlx::query(
        r#"
        SELECT
            id, run_id, turn_step_id, checkpoint_id, reason, control_level, request,
            response, validation_status, visibility, replay_policy, related_task_list_id,
            related_task_item_id, related_docket_task_id, created_at, updated_at
        FROM bear_run_checkpoints
        WHERE run_id = $1
        ORDER BY created_at ASC, checkpoint_id ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(row_to_checkpoint).collect())
}

pub async fn list_checkpoints_for_session(
    pool: &PgPool,
    bear_id: Uuid,
    session_id: &str,
    limit: i64,
) -> Result<Vec<CheckpointArtifactRow>, DenError> {
    let rows = sqlx::query(
        r#"
        SELECT
            c.id, c.run_id, c.turn_step_id, c.checkpoint_id, c.reason, c.control_level,
            c.request, c.response, c.validation_status, c.visibility, c.replay_policy,
            c.related_task_list_id, c.related_task_item_id, c.related_docket_task_id,
            c.created_at, c.updated_at
        FROM bear_run_checkpoints c
        INNER JOIN turn_runs r ON r.run_id = c.run_id
        WHERE r.bear_id = $1 AND r.session_id = $2
        ORDER BY c.created_at DESC, c.checkpoint_id DESC
        LIMIT $3
        "#,
    )
    .bind(bear_id)
    .bind(session_id)
    .bind(limit.max(1))
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(row_to_checkpoint).collect())
}

fn row_to_checkpoint(row: sqlx::postgres::PgRow) -> CheckpointArtifactRow {
    CheckpointArtifactRow {
        id: row.get("id"),
        run_id: row.get("run_id"),
        turn_step_id: row.get("turn_step_id"),
        checkpoint_id: row.get("checkpoint_id"),
        reason: row.get("reason"),
        control_level: row.get("control_level"),
        request: row.get("request"),
        response: row.get("response"),
        validation_status: row.get("validation_status"),
        visibility: row.get("visibility"),
        replay_policy: row.get("replay_policy"),
        related_task_list_id: row.get("related_task_list_id"),
        related_task_item_id: row.get("related_task_item_id"),
        related_docket_task_id: row.get("related_docket_task_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loop::{
        CheckpointEvidenceRef, CheckpointField, CheckpointNextAction, CheckpointReason,
        CheckpointTaskContext, RuntimeCheckpointResponse,
    };
    use den_core::AgentLoopControlLevel;

    async fn seed_run(pool: &PgPool, run_id: &str) -> (Uuid, i32) {
        let suffix = Uuid::new_v4().simple().to_string();
        let username = format!("ckpt{}", &suffix[..12]);
        let email = format!("{username}@example.test");
        let (user_id,): (i32,) = sqlx::query_as(
            r#"
            INSERT INTO users (email, username, display_name, passhash)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        )
        .bind(email)
        .bind(&username)
        .bind("Checkpoint Test")
        .bind("test-passhash")
        .fetch_one(pool)
        .await
        .expect("create user");
        let bear_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO bears (id, slug, name)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(bear_id)
        .bind(format!("checkpoint-bear-{}", &suffix[..12]))
        .bind("Checkpoint Bear")
        .execute(pool)
        .await
        .expect("create bear");
        sqlx::query(
            r#"
            INSERT INTO turn_runs (run_id, session_id, bear_id, user_id, state)
            VALUES ($1, $2, $3, $4, 'running')
            "#,
        )
        .bind(run_id)
        .bind(format!("session-{run_id}"))
        .bind(bear_id)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("create run");
        (bear_id, user_id)
    }

    fn request(run_id: &str) -> RuntimeCheckpointRequest {
        RuntimeCheckpointRequest {
            checkpoint_id: "ckpt-1".to_string(),
            run_id: run_id.to_string(),
            reason: CheckpointReason::OverExploration,
            control_level: AgentLoopControlLevel::Careful,
            active_objective: Some("Find the failing path".to_string()),
            task_context: Some(CheckpointTaskContext {
                task_list_id: Some("list-1".to_string()),
                task_list_version: Some("2".to_string()),
                active_item_id: Some("item-1".to_string()),
                active_item_title: Some("Inspect runtime".to_string()),
                docket_job_id: None,
                docket_task_id: None,
            }),
            evidence_refs: vec![CheckpointEvidenceRef {
                kind: "tool_result".to_string(),
                id: "call-1".to_string(),
                summary: Some("Read runtime file".to_string()),
            }],
            required_fields: vec![CheckpointField::Learned],
        }
    }

    fn response() -> RuntimeCheckpointResponse {
        RuntimeCheckpointResponse {
            checkpoint_id: "ckpt-1".to_string(),
            active_objective: "Find the failing path".to_string(),
            summary: None,
            learned: vec!["The runtime parser is involved.".to_string()],
            remaining_uncertainty: vec![],
            more_exploration_justified: false,
            next_action: CheckpointNextAction::Validate,
            task_state_change_needed: None,
            evidence_refs: vec![],
            confidence: None,
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn records_checkpoint_request_and_response(pool: PgPool) {
        let run_id = format!("run-{}", Uuid::new_v4().simple());
        let (bear_id, _) = seed_run(&pool, &run_id).await;

        let recorded = record_checkpoint_request(
            &pool,
            CheckpointArtifactInput {
                run_id: run_id.clone(),
                turn_step_id: None,
                request: request(&run_id),
                visibility: CheckpointVisibility::AuditOnly,
                replay_policy: CheckpointReplayPolicy::None,
            },
        )
        .await
        .expect("record request");

        assert_eq!(recorded.run_id, run_id);
        assert_eq!(recorded.checkpoint_id, "ckpt-1");
        assert_eq!(recorded.validation_status, "requested");
        assert_eq!(recorded.visibility, "audit_only");
        assert_eq!(recorded.replay_policy, "none");
        assert_eq!(recorded.related_task_list_id.as_deref(), Some("list-1"));
        assert_eq!(recorded.related_task_item_id.as_deref(), Some("item-1"));
        assert!(recorded.response.is_none());

        let updated = record_checkpoint_response(
            &pool,
            CheckpointResponseInput {
                run_id: run_id.clone(),
                checkpoint_id: "ckpt-1".to_string(),
                response: response(),
                validation_status: CheckpointValidationStatus::Valid,
            },
        )
        .await
        .expect("record response");
        assert_eq!(updated.validation_status, "valid");
        assert!(updated.response.is_some());

        let all = list_checkpoints_for_run(&pool, &run_id)
            .await
            .expect("list checkpoints");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].checkpoint_id, "ckpt-1");

        let by_session = list_checkpoints_for_session(&pool, bear_id, &format!("session-{run_id}"), 10)
            .await
            .expect("list checkpoints by session");
        assert_eq!(by_session.len(), 1);
        assert_eq!(by_session[0].checkpoint_id, "ckpt-1");
    }
}
