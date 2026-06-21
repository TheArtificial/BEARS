use crate::acp::workflow_guidance::render_turn_state_summary;
use den_docket::{WorkPlanItemStatus, WorkPlanProjection};
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
    let mut summary = render_turn_state_summary(
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
    );
    if let Some(reminder) = active_activity_plan_status_reminder(activity_plan) {
        summary.push(' ');
        summary.push_str(&reminder);
    }
    summary
}

fn active_activity_plan_status_reminder(
    activity_plan: Option<&WorkPlanProjection>,
) -> Option<String> {
    let plan = activity_plan?;
    if matches!(plan.status.as_str(), "completed" | "cancelled" | "archived") {
        return None;
    }
    let outstanding_count = plan
        .items
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                WorkPlanItemStatus::Pending
                    | WorkPlanItemStatus::InProgress
                    | WorkPlanItemStatus::Blocked
            )
        })
        .count();
    let completed_count = plan
        .items
        .iter()
        .filter(|item| item.status == WorkPlanItemStatus::Completed)
        .count();
    let current_item = plan
        .current_item
        .as_ref()
        .map(|item| item.title.as_str())
        .unwrap_or("none");
    Some(format!(
        "ACTIVE TASK LIST REMINDER: `{}` is frontmost for this ACP session (plan_id={}, status={}, outstanding_items={}, completed_items={}, current_item=`{}`). Keep it current on every turn: when you finish or abandon an item, call `update_plan` promptly to mark it `completed`, `blocked`, or `cancelled` and move the next item to `in_progress` before or while reporting progress. Do not claim a task is complete while the visible task list still shows it pending/in_progress.",
        plan.title,
        plan.id,
        plan.status,
        outstanding_count,
        completed_count,
        current_item,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use den_docket::WorkPlanItem;
    use time::OffsetDateTime;
    use uuid::Uuid;

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
    fn active_activity_plan_reminder_marks_task_list_frontmost() {
        let plan = plan("active");
        let reminder = active_activity_plan_status_reminder(Some(&plan)).expect("reminder");

        assert!(reminder.contains("ACTIVE TASK LIST REMINDER"));
        assert!(reminder.contains("frontmost for this ACP session"));
        assert!(reminder.contains("call `update_plan` promptly"));
        assert!(reminder.contains("Do not claim a task is complete"));
        assert!(reminder.contains("outstanding_items=2"));
        assert!(reminder.contains("completed_items=1"));
    }

    #[test]
    fn terminal_activity_plan_does_not_emit_status_reminder() {
        let plan = plan("completed");

        assert!(active_activity_plan_status_reminder(Some(&plan)).is_none());
        assert!(active_activity_plan_status_reminder(None).is_none());
    }
}
