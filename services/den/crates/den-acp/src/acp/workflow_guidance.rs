#[derive(Debug, Clone, Copy)]
pub(super) struct ActivityStatusSyncSummary {
    pub(super) required: bool,
    pub(super) outstanding_items: u64,
    pub(super) completed_items: u64,
}

pub(super) fn render_turn_state_summary(
    session_id: &str,
    roots: &[String],
    local_tool_names: &[&str],
    den_tool_names: &[&str],
    policy_mode_label: &str,
    allowed_tool_classes: &[&str],
    workplan_state: &str,
    workplan_approval_status: &str,
    activity_status: &str,
    activity_plan_id: &str,
    current_item: &str,
    execution_unlocked: bool,
    activity_status_sync: ActivityStatusSyncSummary,
) -> String {
    let activity_status_sync_text = if activity_status_sync.required {
        format!(
            " activity.status_sync_required=true; activity.status_update_tool=`update_plan`; activity.outstanding_items={}; activity.completed_items={}; completion_claim_requires_status_update=true. When you finish, block, or abandon an activity item, update the visible task list promptly with `update_plan` before claiming that item is complete; move the next item to `in_progress` when appropriate.",
            activity_status_sync.outstanding_items,
            activity_status_sync.completed_items,
        )
    } else {
        " activity.status_sync_required=false;".to_string()
    };
    format!(
        "<system-reminder>AUTHORITATIVE WORKFLOW STATE for this turn: permission_mode=`{}`; tool_classes={}; workplan.state=`{}`; workplan.approval_status={}; activity.status=`{}`; activity.plan_id=`{}`; activity.current_item=`{}`;{} memory.active_plan_write_allowed=false; execution.execution_unlocked={}; state_authority=current turn capabilities override prior-turn assumptions. BEARS ACP direct local workspace tools available this turn: {}. Server tools available to pair: {}. Current ACP session id is `{}`. Use absolute paths under these workspace roots: {}.</system-reminder>",
        policy_mode_label,
        allowed_tool_classes.join(", "),
        workplan_state,
        workplan_approval_status,
        activity_status,
        activity_plan_id,
        current_item,
        activity_status_sync_text,
        execution_unlocked,
        local_tool_names.join(", "),
        den_tool_names.join(", "),
        session_id,
        roots.join(", "),
    )
}
