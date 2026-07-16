//! Runtime focus resolution.
//!
//! Durable Docket execution is the source of truth for focused work. The
//! `AgentLoopSession.cached_activity_plan_projection` field is only a volatile
//! projection cache used to seed prompts/tools and avoid rehydrating Docket on
//! every stream event.
//!
//! Runtime behavior decisions, diagnostics, and user-facing status projections
//! should resolve a `RuntimeFocusContext` instead of reading the session cache as
//! authoritative.

use den_core::DenError;
use den_docket::{
    DocketExecutionLookup, DocketService, PgDocketService, TaskListCheckoutRequest,
    TaskListCheckoutSource, TaskListProjection,
};
use den_service::bears::BearProfile;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFocusSource {
    DurableDocketExecution,
    RuntimeCachedActivityPlan,
    None,
}

impl RuntimeFocusSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DurableDocketExecution => "durable_docket_execution",
            Self::RuntimeCachedActivityPlan => "runtime_cached_activity_plan",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeFocusContext {
    pub source: RuntimeFocusSource,
    pub cached_activity_plan_projection: Option<TaskListProjection>,
}

impl RuntimeFocusContext {
    pub fn active_activity_plan(&self) -> Option<&TaskListProjection> {
        self.cached_activity_plan_projection.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeFocusResolveRequest {
    pub bear_id: Uuid,
    pub profile: BearProfile,
    pub user_id: Option<i32>,
    pub conversation_id: String,
    pub client_session_id: String,
    pub cached_activity_plan_projection: Option<TaskListProjection>,
}

pub fn active_docket_execution_lookup(
    session_id: Option<&str>,
    conversation_id: &str,
) -> DocketExecutionLookup {
    DocketExecutionLookup {
        session_id: session_id.map(str::to_string),
        // ponytail: conversation-scoped focus is the durable restore path for now; upgrade to an
        // explicit conversation focus record if focus needs history, labels, or multi-job stacks.
        source_conversation_id: Some(conversation_id.to_string()),
        source_client_session_id: session_id.map(str::to_string),
    }
}

pub fn active_docket_execution_lookup_for_session(
    conversation_id: &str,
    client_session_id: &str,
) -> DocketExecutionLookup {
    active_docket_execution_lookup(Some(client_session_id), conversation_id)
}

pub async fn resolve_runtime_focus_context(
    pool: &PgPool,
    request: RuntimeFocusResolveRequest,
) -> Result<RuntimeFocusContext, DenError> {
    let RuntimeFocusResolveRequest {
        bear_id,
        profile,
        user_id,
        conversation_id,
        client_session_id,
        cached_activity_plan_projection,
    } = request;

    if let Some(user_id) = user_id {
        let service = PgDocketService::from_pool(pool);
        if let Some(execution) = service
            .get_active_execution_session(
                bear_id,
                profile,
                active_docket_execution_lookup_for_session(&conversation_id, &client_session_id),
            )
            .await?
        {
            let plan = service
                .checkout_task_list(
                    bear_id,
                    profile,
                    user_id,
                    TaskListCheckoutRequest {
                        source: TaskListCheckoutSource::DocketJob {
                            job_id: execution.job_id,
                            parent_task_id: None,
                        },
                    },
                )
                .await?;
            return Ok(RuntimeFocusContext {
                source: RuntimeFocusSource::DurableDocketExecution,
                cached_activity_plan_projection: plan,
            });
        }
    }

    if cached_activity_plan_projection.is_some() {
        return Ok(RuntimeFocusContext {
            source: RuntimeFocusSource::RuntimeCachedActivityPlan,
            cached_activity_plan_projection,
        });
    }

    Ok(RuntimeFocusContext {
        source: RuntimeFocusSource::None,
        cached_activity_plan_projection: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_docket_execution_lookup_uses_conversation_focus_restore_path() {
        let lookup = active_docket_execution_lookup_for_session("conv-1", "session-1");
        assert_eq!(lookup.session_id.as_deref(), Some("session-1"));
        assert_eq!(lookup.source_conversation_id.as_deref(), Some("conv-1"));
        assert_eq!(
            lookup.source_client_session_id.as_deref(),
            Some("session-1")
        );
    }
}
