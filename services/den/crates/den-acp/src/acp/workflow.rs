use crate::acp::workflow_guidance::{render_turn_state_summary, ActivityStatusSyncSummary};
use den_docket::WorkPlanProjection;
use den_runtime::{client_tools::ResolvedSessionPolicy, turn_state};

pub(crate) fn workflow_state_json(policy: &ResolvedSessionPolicy) -> serde_json::Value {
    workflow_state_json_from_sources(policy, None, None)
}

pub(crate) fn workflow_state_json_with_activity(
    policy: &ResolvedSessionPolicy,
    activity_plan: Option<&WorkPlanProjection>,
) -> serde_json::Value {
    workflow_state_json_from_sources(policy, None, activity_plan)
}

pub(crate) fn workflow_state_json_from_sources(
    policy: &ResolvedSessionPolicy,
    workplan_row: Option<&den_runtime::plan_mode::PlanModeSessionRow>,
    activity_plan: Option<&WorkPlanProjection>,
) -> serde_json::Value {
    turn_state::turn_state_from_sources(policy, workplan_row, activity_plan)
}

pub(super) fn render_turn_state_summary_with_activity(
    session_id: &str,
    roots: &[String],
    local_tool_names: &[&str],
    den_tool_names: &[&str],
    policy: &ResolvedSessionPolicy,
    activity_plan: Option<&WorkPlanProjection>,
) -> String {
    let execution_unlocked = policy.tool_enablement.enables_non_read_tools();
    let turn_state = workflow_state_json_with_activity(policy, activity_plan);
    let activity_status = turn_state["activity"]["status"]
        .as_str()
        .unwrap_or("inactive");
    let activity_plan_id = turn_state["activity"]["plan_id"].as_str().unwrap_or("none");
    let current_item = turn_state["activity"]["current_item"]["title"]
        .as_str()
        .unwrap_or("none");
    let pending_count = turn_state["activity"]["counts"]["pending"]
        .as_u64()
        .unwrap_or(0);
    let in_progress_count = turn_state["activity"]["counts"]["in_progress"]
        .as_u64()
        .unwrap_or(0);
    let blocked_count = turn_state["activity"]["counts"]["blocked"]
        .as_u64()
        .unwrap_or(0);
    let activity_status_sync = ActivityStatusSyncSummary {
        required: turn_state["activity"]["status_sync_required"]
            .as_bool()
            .unwrap_or(false),
        outstanding_items: pending_count + in_progress_count + blocked_count,
        completed_items: turn_state["activity"]["counts"]["completed"]
            .as_u64()
            .unwrap_or(0),
    };
    render_turn_state_summary(
        session_id,
        roots,
        local_tool_names,
        den_tool_names,
        policy.mode_label,
        &policy.allowed_tool_classes(),
        turn_state["workplan"]["state"]
            .as_str()
            .unwrap_or("inactive"),
        turn_state["workplan"]["approval_status"]
            .as_str()
            .unwrap_or("inactive"),
        activity_status,
        activity_plan_id,
        current_item,
        execution_unlocked,
        activity_status_sync,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use den_docket::{WorkPlanItem, WorkPlanItemStatus};
    use den_runtime::client_tools::ToolEnablementState;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn policy() -> ResolvedSessionPolicy {
        ResolvedSessionPolicy {
            mode_label: "Write",
            tool_enablement: ToolEnablementState::AllTools,
            plan_mode_state: Some("approved".to_string()),
        }
    }

    fn item(id: &str, title: &str, status: WorkPlanItemStatus) -> WorkPlanItem {
        WorkPlanItem {
            id: id.to_string(),
            title: title.to_string(),
            summary: None,
            status,
            blocked_reason: None,
            source_refs: Vec::new(),
        }
    }

    fn plan(status: &str) -> WorkPlanProjection {
        let current = item(
            "patch",
            "Patch task-list reminder",
            WorkPlanItemStatus::InProgress,
        );
        WorkPlanProjection {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap(),
            bear_id: Uuid::parse_str("00000000-0000-0000-0000-000000000456").unwrap(),
            title: "Fix ACP planning visibility".to_string(),
            summary: "Keep task statuses current".to_string(),
            owner_profile: "pair".to_string(),
            visibility: "same_user".to_string(),
            status: status.to_string(),
            version: 1,
            items: vec![
                item("inspect", "Inspect logs", WorkPlanItemStatus::Completed),
                current.clone(),
                item("test", "Run tests", WorkPlanItemStatus::Pending),
            ],
            current_item: Some(current),
            source_conversation_id: None,
            source_acp_session_id: Some("acp-test".to_string()),
            handoff_intent_path: None,
            handoff_task_id: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn active_activity_plan_projects_status_sync_requirement_from_turn_state() {
        let plan = plan("active");
        let summary = render_turn_state_summary_with_activity(
            "acp-test",
            &["/workspace".to_string()],
            &[],
            &["update_plan"],
            &policy(),
            Some(&plan),
        );

        assert!(summary.contains("activity.status_sync_required=true"));
        assert!(summary.contains("activity.status_update_tool=`update_plan`"));
        assert!(summary.contains("activity.outstanding_items=2"));
        assert!(summary.contains("activity.completed_items=1"));
        assert!(summary.contains("completion_claim_requires_status_update=true"));
    }

    #[test]
    fn terminal_or_absent_activity_plan_projects_no_status_sync_requirement() {
        let plan = plan("completed");
        let terminal = render_turn_state_summary_with_activity(
            "acp-test",
            &["/workspace".to_string()],
            &[],
            &["update_plan"],
            &policy(),
            Some(&plan),
        );
        let absent = render_turn_state_summary_with_activity(
            "acp-test",
            &["/workspace".to_string()],
            &[],
            &["update_plan"],
            &policy(),
            None,
        );

        assert!(terminal.contains("activity.status_sync_required=false"));
        assert!(absent.contains("activity.status_sync_required=false"));
        assert!(!terminal.contains("completion_claim_requires_status_update=true"));
        assert!(!absent.contains("completion_claim_requires_status_update=true"));
    }
}
