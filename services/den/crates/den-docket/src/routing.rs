//! ADR-0056's single durable turn-placement seam.
//!
//! This module decides transcript containment. Docket run/task state remains
//! the sole authority for execution position; a cursor is only a viewport.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use den_core::DenError;

use crate::model::{DocketTaskDifficulty, RoutingStrategy};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnSource {
    User,
    Continuation,
    Dispatch,
    Rollup,
}
impl TurnSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Continuation => "continuation",
            Self::Dispatch => "dispatch",
            Self::Rollup => "rollup",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSurface {
    Sandbox,
    Armature,
}
impl ExecutionSurface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Armature => "armature",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStrategy {
    Reuse,
    Inline,
    Scoped,
    Delegated,
}
impl ConversationStrategy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reuse => "reuse",
            Self::Inline => "inline",
            Self::Scoped => "scoped",
            Self::Delegated => "delegated",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TurnIntent {
    pub idempotency_key: Uuid,
    pub bear_id: Uuid,
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub task_id: Uuid,
    pub source: TurnSource,
    pub originating_conversation_id: Option<String>,
    pub parent_conversation_id: Option<String>,
    pub surface: ExecutionSurface,
    pub resolved_profile: Option<String>,
    pub attempt: i32,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub struct RoutingDecision {
    pub id: Uuid,
    pub idempotency_key: Uuid,
    pub bear_id: Uuid,
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub task_id: Uuid,
    pub turn_source: String,
    pub conversation_strategy: String,
    pub conversation_id: String,
    pub parent_conversation_id: Option<String>,
    pub routing_strategy: String,
    pub execution_surface: String,
    pub resolved_profile: Option<String>,
    pub attempt: i32,
    pub cursor_before: Option<Value>,
    pub cursor_after: Option<Value>,
    pub reason: String,
    pub created_at: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct TaskRoutingRow {
    routing_strategy: String,
    expected_context_size: Option<i32>,
    difficulty: Option<String>,
}

/// Resolve and persist placement before invoking a model. Replaying the same
/// idempotency key returns the original immutable decision.
pub async fn route_turn(pool: &PgPool, intent: TurnIntent) -> Result<RoutingDecision, DenError> {
    if intent.attempt < 1 {
        return Err(DenError::ValidationError(
            "turn attempt must be positive".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    if let Some(existing) = sqlx::query_as!(
        RoutingDecision,
        r#"
        SELECT
            id AS "id!: Uuid",
            idempotency_key AS "idempotency_key!: Uuid",
            bear_id AS "bear_id!: Uuid",
            job_id AS "job_id!: Uuid",
            run_id AS "run_id!: Uuid",
            task_id AS "task_id!: Uuid",
            turn_source AS "turn_source!: String",
            conversation_strategy AS "conversation_strategy!: String",
            conversation_id AS "conversation_id!: String",
            parent_conversation_id AS "parent_conversation_id?: String",
            routing_strategy AS "routing_strategy!: String",
            execution_surface AS "execution_surface!: String",
            resolved_profile AS "resolved_profile?: String",
            attempt AS "attempt!: i32",
            cursor_before AS "cursor_before?: Value",
            cursor_after AS "cursor_after?: Value",
            reason AS "reason!: String",
            created_at AS "created_at!: OffsetDateTime"
        FROM docket_routing_decisions
        WHERE idempotency_key = $1
        "#,
        intent.idempotency_key
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(existing);
    }
    let task = sqlx::query_as!(
        TaskRoutingRow,
        r#"
        SELECT
            routing_strategy AS "routing_strategy!: String",
            expected_context_size AS "expected_context_size?: i32",
            difficulty AS "difficulty?: String"
        FROM bear_tasks
        WHERE id = $1 AND job_id = $2 AND bear_id = $3
        FOR SHARE
        "#,
        intent.task_id,
        intent.job_id,
        intent.bear_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(TaskRoutingRow {
        routing_strategy: raw_strategy,
        expected_context_size,
        difficulty,
    }) = task
    else {
        return Err(DenError::NotFound(format!(
            "Docket task not found: {}",
            intent.task_id
        )));
    };
    let configured = parse_routing_strategy(&raw_strategy)?;
    let binding: Option<String> = sqlx::query_scalar!(
        r#"
        SELECT preferred_conversation_id AS "preferred_conversation_id!: String"
        FROM docket_conversation_bindings
        WHERE task_id = $1
        "#,
        intent.task_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    let (strategy, conversation_id, reason) = if let Some(bound) = binding {
        (
            ConversationStrategy::Reuse,
            bound,
            "reuse existing task conversation",
        )
    } else {
        let resolved = resolve_strategy(configured, expected_context_size, difficulty.as_deref());
        let conversation = match resolved {
            ConversationStrategy::Inline => intent
                .originating_conversation_id
                .clone()
                .or_else(|| intent.parent_conversation_id.clone()),
            ConversationStrategy::Scoped
            | ConversationStrategy::Delegated
            | ConversationStrategy::Reuse => None,
        }
        .unwrap_or_else(|| format!("den-conv-{}", Uuid::new_v4().simple()));
        let reason = match resolved {
            ConversationStrategy::Inline => "continue in originating conversation",
            ConversationStrategy::Scoped => "isolate non-trivial task context",
            ConversationStrategy::Delegated => "use delegated task context",
            ConversationStrategy::Reuse => unreachable!(),
        };
        (resolved, conversation, reason)
    };
    if !matches!(strategy, ConversationStrategy::Inline) {
        sqlx::query!(
            r#"
            INSERT INTO docket_conversation_bindings (task_id, preferred_conversation_id)
            VALUES ($1, $2)
            ON CONFLICT (task_id) DO NOTHING
            "#,
            intent.task_id,
            &conversation_id
        )
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query!(
        r#"
        INSERT INTO docket_conversation_binding_runs (run_id, task_id, conversation_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (run_id, task_id) DO NOTHING
        "#,
        intent.run_id,
        intent.task_id,
        &conversation_id
    )
    .execute(&mut *tx)
    .await?;
    let decision = sqlx::query_as!(
        RoutingDecision,
        r#"
        INSERT INTO docket_routing_decisions (
            idempotency_key, bear_id, job_id, run_id, task_id, turn_source,
            conversation_strategy, conversation_id, parent_conversation_id,
            routing_strategy, execution_surface, resolved_profile, attempt, reason
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        RETURNING
            id AS "id!: Uuid",
            idempotency_key AS "idempotency_key!: Uuid",
            bear_id AS "bear_id!: Uuid",
            job_id AS "job_id!: Uuid",
            run_id AS "run_id!: Uuid",
            task_id AS "task_id!: Uuid",
            turn_source AS "turn_source!: String",
            conversation_strategy AS "conversation_strategy!: String",
            conversation_id AS "conversation_id!: String",
            parent_conversation_id AS "parent_conversation_id?: String",
            routing_strategy AS "routing_strategy!: String",
            execution_surface AS "execution_surface!: String",
            resolved_profile AS "resolved_profile?: String",
            attempt AS "attempt!: i32",
            cursor_before AS "cursor_before?: Value",
            cursor_after AS "cursor_after?: Value",
            reason AS "reason!: String",
            created_at AS "created_at!: OffsetDateTime"
        "#,
        intent.idempotency_key,
        intent.bear_id,
        intent.job_id,
        intent.run_id,
        intent.task_id,
        intent.source.as_str(),
        strategy.as_str(),
        &conversation_id,
        intent.parent_conversation_id.as_deref(),
        configured.as_str(),
        intent.surface.as_str(),
        intent.resolved_profile.as_deref(),
        intent.attempt,
        reason
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(decision)
}

fn parse_routing_strategy(raw: &str) -> Result<RoutingStrategy, DenError> {
    match raw {
        "inline" => Ok(RoutingStrategy::Inline),
        "scoped" => Ok(RoutingStrategy::Scoped),
        "delegated" => Ok(RoutingStrategy::Delegated),
        "auto" => Ok(RoutingStrategy::Auto),
        _ => Err(DenError::ValidationError(format!(
            "unknown routing strategy: {raw}"
        ))),
    }
}

fn resolve_strategy(
    configured: RoutingStrategy,
    expected_context_size: Option<i32>,
    difficulty: Option<&str>,
) -> ConversationStrategy {
    match configured {
        RoutingStrategy::Inline => ConversationStrategy::Inline,
        RoutingStrategy::Scoped => ConversationStrategy::Scoped,
        RoutingStrategy::Delegated => ConversationStrategy::Delegated,
        RoutingStrategy::Auto
            if difficulty == Some(DocketTaskDifficulty::Trivial.as_str())
                && expected_context_size.unwrap_or(0) <= 8_000 =>
        {
            ConversationStrategy::Inline
        }
        RoutingStrategy::Auto => ConversationStrategy::Scoped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_only_inlines_small_trivial_tasks() {
        assert_eq!(
            resolve_strategy(RoutingStrategy::Auto, Some(100), Some("trivial")),
            ConversationStrategy::Inline
        );
        assert_eq!(
            resolve_strategy(RoutingStrategy::Auto, Some(9000), Some("trivial")),
            ConversationStrategy::Scoped
        );
        assert_eq!(
            resolve_strategy(RoutingStrategy::Auto, None, Some("moderate")),
            ConversationStrategy::Scoped
        );
    }
}
