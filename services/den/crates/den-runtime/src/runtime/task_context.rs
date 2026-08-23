//! Runtime current-task resolution.
//!
//! Pair resolves only its persisted current-task selection. The task-list
//! projection is a volatile cache used to seed prompts and tools; runtime
//! behavior resolves persisted state rather than treating cached or legacy
//! execution state as authoritative.

use den_core::DenError;
use den_docket::{
    task_list_projection_from_session_tasks_with_current_task, DocketService, PgDocketService,
    TaskListProjection,
};
use den_service::{bears::BearProfile, client_sessions};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeTaskSource {
    SessionCurrentTask,
    None,
}

impl RuntimeTaskSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionCurrentTask => "session_current_task",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeTaskContext {
    pub source: RuntimeTaskSource,
    /// The explicitly selected task for this session, when one exists.
    ///
    /// This remains separate from the task-list projection: the projection
    /// supplies surrounding task-tree context, while this ID identifies the
    /// task Pair should treat as its current objective.
    pub current_task_id: Option<Uuid>,
    pub cached_activity_plan_projection: Option<TaskListProjection>,
}

impl RuntimeTaskContext {
    pub fn active_activity_plan(&self) -> Option<&TaskListProjection> {
        match self.source {
            RuntimeTaskSource::SessionCurrentTask => self.cached_activity_plan_projection.as_ref(),
            RuntimeTaskSource::None => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeTaskResolveRequest {
    pub bear_id: Uuid,
    pub profile: BearProfile,
    pub user_id: Option<i32>,
    pub conversation_id: String,
    pub client_session_id: String,
    pub cached_activity_plan_projection: Option<TaskListProjection>,
}

fn is_actionable_session_task_status(status: den_docket::DocketTaskStatus) -> bool {
    status == den_docket::DocketTaskStatus::Pending
}

pub async fn resolve_runtime_task_context(
    pool: &PgPool,
    request: RuntimeTaskResolveRequest,
) -> Result<RuntimeTaskContext, DenError> {
    let RuntimeTaskResolveRequest {
        bear_id,
        profile,
        user_id,
        conversation_id,
        client_session_id,
        cached_activity_plan_projection: _,
    } = request;

    let Some(user_id) = user_id else {
        return Ok(RuntimeTaskContext {
            source: RuntimeTaskSource::None,
            current_task_id: None,
            cached_activity_plan_projection: None,
        });
    };
    let Some(session) =
        client_sessions::find_for_user_bear_session_id(pool, user_id, bear_id, &client_session_id)
            .await?
    else {
        return Ok(RuntimeTaskContext {
            source: RuntimeTaskSource::None,
            current_task_id: None,
            cached_activity_plan_projection: None,
        });
    };
    let service = PgDocketService::from_pool(pool);
    let tasks = service.list_pair_session_tasks(bear_id, session.id).await?;
    let current_task_id = session.current_task_id.filter(|selected_task_id| {
        tasks
            .iter()
            .find(|task| task.task.id == *selected_task_id)
            .is_some_and(|task| is_actionable_session_task_status(task.status))
    });
    if current_task_id.is_some() {
        let plan = task_list_projection_from_session_tasks_with_current_task(
            bear_id,
            profile,
            &conversation_id,
            session.id,
            &tasks,
            current_task_id,
        );
        return Ok(RuntimeTaskContext {
            source: RuntimeTaskSource::SessionCurrentTask,
            current_task_id,
            cached_activity_plan_projection: plan,
        });
    }

    let plan = task_list_projection_from_session_tasks_with_current_task(
        bear_id,
        profile,
        &conversation_id,
        session.id,
        &tasks,
        current_task_id,
    );
    Ok(RuntimeTaskContext {
        source: if current_task_id.is_some() || plan.is_some() {
            RuntimeTaskSource::SessionCurrentTask
        } else {
            RuntimeTaskSource::None
        },
        current_task_id,
        cached_activity_plan_projection: plan,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_session_tasks_are_not_current_task_candidates() {
        assert!(is_actionable_session_task_status(
            den_docket::DocketTaskStatus::Pending
        ));
        assert!(!is_actionable_session_task_status(
            den_docket::DocketTaskStatus::Done
        ));
        assert!(!is_actionable_session_task_status(
            den_docket::DocketTaskStatus::Blocked
        ));
        assert!(!is_actionable_session_task_status(
            den_docket::DocketTaskStatus::Cancelled
        ));
    }

    #[test]
    fn session_current_task_exposes_its_projection() {
        let context = RuntimeTaskContext {
            source: RuntimeTaskSource::SessionCurrentTask,
            current_task_id: None,
            cached_activity_plan_projection: None,
        };
        assert!(context.active_activity_plan().is_none());
        assert_eq!(context.source.as_str(), "session_current_task");
    }

    #[test]
    fn durable_execution_prefers_its_persisted_task() {
        let task_id = Uuid::new_v4();
        assert_eq!(
            durable_execution_current_task_id(Some(task_id), None),
            Some(task_id)
        );
    }

    #[test]
    fn active_docket_execution_lookup_uses_conversation_execution_restore_path() {
        let lookup = active_docket_execution_lookup_for_session("conv-1", "session-1");
        assert_eq!(lookup.session_id.as_deref(), Some("session-1"));
        assert_eq!(lookup.source_conversation_id.as_deref(), Some("conv-1"));
        assert_eq!(
            lookup.source_client_session_id.as_deref(),
            Some("session-1")
        );
    }
}
