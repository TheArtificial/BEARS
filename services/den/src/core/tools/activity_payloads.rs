use serde_json::{json, Value};

use den_docket::WorkPlanProjection;
use den_runtime::{plan_mode, turn_state};

pub(crate) fn plan_mode_workplan_payload(row: &plan_mode::PlanModeSessionRow) -> Value {
    turn_state::turn_state_from_sources(
        &den_runtime::client_tools::ResolvedSessionPolicy {
            mode_label: if row.state == "approved" {
                "Write"
            } else {
                "Plan"
            },
            tool_enablement: if row.state == "approved" {
                den_runtime::client_tools::ToolEnablementState::AllTools
            } else {
                den_runtime::client_tools::ToolEnablementState::ReadOnly
            },
            plan_mode_state: Some(row.state.clone()),
        },
        Some(row),
        None,
    )["workplan"]
        .clone()
}

pub(crate) fn no_active_workplan_payload() -> Value {
    json!({
        "domain": "workplan",
        "plan_id": Value::Null,
        "id": Value::Null,
        "state": "inactive",
        "approval_status": "inactive",
        "raw_state": Value::Null,
        "submitted_plan_present": false,
        "artifact_path": Value::Null,
        "title": Value::Null,
        "summary": Value::Null,
        "execution_unlocked": false,
    })
}

pub(crate) fn activity_payload(plan: Option<&WorkPlanProjection>) -> Value {
    match plan {
        Some(plan) => json!({
            "domain": "activity",
            "plan_id": plan.id,
            "id": plan.id,
            "status": plan.status.clone(),
            "title": plan.title.clone(),
            "summary": plan.summary.clone(),
            "current_item": plan.current_item.clone(),
            "items": plan.items.clone(),
            "visibility": plan.visibility.clone(),
            "owner_profile": plan.owner_profile.clone(),
            "version": plan.version,
            "handoff_requested": plan.handoff_intent_path.is_some() || plan.handoff_task_id.is_some(),
            "handoff_intent_path": plan.handoff_intent_path.clone(),
            "handoff_task_id": plan.handoff_task_id.clone(),
            "updated_at": plan.updated_at,
        }),
        None => json!({
            "domain": "activity",
            "plan_id": Value::Null,
            "id": Value::Null,
            "status": "inactive",
            "title": Value::Null,
            "summary": Value::Null,
            "current_item": Value::Null,
            "items": [],
            "handoff_requested": false,
        }),
    }
}
