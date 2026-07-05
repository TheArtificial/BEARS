use std::str::FromStr;

use serde_json::{Value, json};

use den_core::{client_tools::ResolvedSessionPolicy, profile::BearProfile};
use crate::plan_mode;
use den_docket::{TaskListItem, TaskListProjection, WorkPlanItem, WorkPlanItemStatus, WorkPlanProjection};

pub const TURN_STATE_SCHEMA: &str = "bears.turn_state/v1";
pub const TURN_STATE_VERSION: u32 = 1;
pub const TURN_STATE_AUTHORITY: &str = "current_turn_capabilities";

const AUTONOMOUS_CONTINUATION_POLICY: &str = "continue_until_complete_or_blocked";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonomousFinalResponseKind {
    CompletionFinal,
    BlockedFinal,
    ReasonedNonActionFinal,
    ProgressReport,
    ClarificationRequest,
    UnsafeActionPermissionRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutonomousExecutionGate {
    pub is_active_autonomous_task: bool,
    pub has_incomplete_unblocked_items: bool,
    pub acceptance_criteria_met: bool,
    pub has_hard_blocker: bool,
    pub may_stop: bool,
    pub next_incomplete_task_title: Option<String>,
}

pub fn workflow_state_label(policy: &ResolvedSessionPolicy) -> &'static str {
    match policy.plan_mode_state.as_deref() {
        Some("submitted") => "submitted_waiting_approval",
        Some("approved") => "approved",
        Some("active") => "drafting",
        Some("rejected") => "cancelled",
        _ if policy.mode_label == "Write" => "approved",
        _ => "inactive",
    }
}

pub fn approval_status_label(plan_mode_state: Option<&str>, mode_label: &str) -> &'static str {
    match plan_mode_state {
        Some("approved") => "approved_execution_unlocked",
        Some("submitted") => "awaiting_human_approval",
        Some("active") => "drafting",
        Some("rejected") => "cancelled",
        _ if mode_label == "Write" => "approved_execution_unlocked",
        _ => "inactive",
    }
}

pub fn turn_state_json(
    policy: &ResolvedSessionPolicy,
    activity_plan: Option<&WorkPlanProjection>,
) -> Value {
    turn_state_from_sources(policy, None, activity_plan)
}

pub fn turn_state_from_sources(
    policy: &ResolvedSessionPolicy,
    workplan_row: Option<&plan_mode::PlanModeSessionRow>,
    activity_plan: Option<&WorkPlanProjection>,
) -> Value {
    let workplan = workplan_domain_json(policy, workplan_row);
    let activity = activity_domain_json(activity_plan);
    let autonomous_execution = autonomous_execution_domain_json(activity_plan);
    json!({
        "schema": TURN_STATE_SCHEMA,
        "state_version": TURN_STATE_VERSION,
        "state_authority": TURN_STATE_AUTHORITY,
        "focus": {
            "current_domain": if activity_plan.is_some() { "activity" } else { "workplan" },
            "current_activity_id": activity["plan_id"].clone(),
            "current_workplan_id": workplan["plan_id"].clone(),
            "root_workplan_id": workplan["root_id"].clone(),
        },
        "workplan": workplan,
        "activity": activity,
        "autonomous_execution": autonomous_execution,
        "memory": memory_domain_json(),
        "execution": execution_domain_json(policy),
    })
}

pub fn autonomous_execution_gate_for_plan(
    profile: BearProfile,
    plan: Option<&WorkPlanProjection>,
    final_response_kind: AutonomousFinalResponseKind,
) -> AutonomousExecutionGate {
    let Some(plan) = plan.filter(|plan| is_autonomous_implementation_plan(profile, plan)) else {
        return AutonomousExecutionGate {
            is_active_autonomous_task: false,
            has_incomplete_unblocked_items: false,
            acceptance_criteria_met: false,
            has_hard_blocker: false,
            may_stop: true,
            next_incomplete_task_title: None,
        };
    };

    let items = &plan.items;
    let acceptance_criteria_met = acceptance_criteria_met(plan);
    let has_hard_blocker = items.iter().any(|item| item.status == WorkPlanItemStatus::Blocked)
        || matches!(plan.status.as_str(), "blocked");
    let next_incomplete_task_title = next_incomplete_unblocked_item(items).map(|item| item.title.clone());
    let has_incomplete_unblocked_items = next_incomplete_task_title.is_some();
    let may_stop = if acceptance_criteria_met {
        final_response_kind == AutonomousFinalResponseKind::CompletionFinal
    } else if has_hard_blocker {
        matches!(
            final_response_kind,
            AutonomousFinalResponseKind::BlockedFinal
                | AutonomousFinalResponseKind::ReasonedNonActionFinal
                | AutonomousFinalResponseKind::UnsafeActionPermissionRequest
        )
    } else if !has_incomplete_unblocked_items {
        matches!(
            final_response_kind,
            AutonomousFinalResponseKind::CompletionFinal
                | AutonomousFinalResponseKind::ReasonedNonActionFinal
        )
    } else {
        false
    };

    AutonomousExecutionGate {
        is_active_autonomous_task: true,
        has_incomplete_unblocked_items,
        acceptance_criteria_met,
        has_hard_blocker,
        may_stop,
        next_incomplete_task_title,
    }
}

pub fn classify_autonomous_final_response(text: &str) -> AutonomousFinalResponseKind {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("need approval")
        || lower.contains("requires approval")
        || lower.contains("permission")
    {
        return AutonomousFinalResponseKind::UnsafeActionPermissionRequest;
    }
    if lower.contains("not applicable")
        || lower.contains("waived")
        || lower.contains("intentionally skipped")
        || lower.contains("skipped because")
        || lower.contains("not appropriate")
        || lower.contains("no relevant changes")
        || lower.contains("do not commit")
        || lower.contains("did not commit")
    {
        return AutonomousFinalResponseKind::ReasonedNonActionFinal;
    }
    if lower.contains("blocked") || lower.contains("cannot continue") {
        return AutonomousFinalResponseKind::BlockedFinal;
    }
    if lower.contains("what i changed")
        || lower.contains("remaining work")
        || lower.contains("next useful steps")
        || lower.contains("next i would")
        || lower.contains("i can continue if")
    {
        // heuristic progress-report detection only catches common partial-summary shapes;
        // replace with structured assistant turn intent once final responses are explicitly typed.
        return AutonomousFinalResponseKind::ProgressReport;
    }
    if lower.ends_with('?') {
        return AutonomousFinalResponseKind::ClarificationRequest;
    }
    AutonomousFinalResponseKind::CompletionFinal
}

pub fn should_allow_terminal_response(
    profile: BearProfile,
    active_activity_plan: Option<&WorkPlanProjection>,
    assistant_text: &str,
) -> bool {
    let kind = classify_autonomous_final_response(assistant_text);
    autonomous_execution_gate_for_plan(profile, active_activity_plan, kind).may_stop
}

pub fn autonomous_execution_gate_for_task_list(
    profile: BearProfile,
    task_list: Option<&TaskListProjection>,
    final_response_kind: AutonomousFinalResponseKind,
) -> AutonomousExecutionGate {
    let Some(task_list) = task_list.filter(|task_list| is_autonomous_task_list(profile, task_list)) else {
        return AutonomousExecutionGate {
            is_active_autonomous_task: false,
            has_incomplete_unblocked_items: false,
            acceptance_criteria_met: false,
            has_hard_blocker: false,
            may_stop: true,
            next_incomplete_task_title: None,
        };
    };

    let acceptance_criteria_met = task_list_acceptance_criteria_met(task_list);
    let has_hard_blocker = task_list
        .items
        .iter()
        .any(|item| item.status == WorkPlanItemStatus::Blocked)
        || matches!(task_list.status.as_str(), "blocked");
    let next_incomplete_task_title = next_incomplete_unblocked_task_list_item(&task_list.items)
        .map(|item| item.title.clone());
    let has_incomplete_unblocked_items = next_incomplete_task_title.is_some();
    let may_stop = if acceptance_criteria_met {
        final_response_kind == AutonomousFinalResponseKind::CompletionFinal
    } else if has_hard_blocker {
        matches!(
            final_response_kind,
            AutonomousFinalResponseKind::BlockedFinal
                | AutonomousFinalResponseKind::ReasonedNonActionFinal
                | AutonomousFinalResponseKind::UnsafeActionPermissionRequest
        )
    } else if has_incomplete_unblocked_items {
        false
    } else {
        matches!(
            final_response_kind,
            AutonomousFinalResponseKind::CompletionFinal
                | AutonomousFinalResponseKind::ReasonedNonActionFinal
        )
    };

    AutonomousExecutionGate {
        is_active_autonomous_task: true,
        has_incomplete_unblocked_items,
        acceptance_criteria_met,
        has_hard_blocker,
        may_stop,
        next_incomplete_task_title,
    }
}

pub fn should_allow_terminal_response_for_task_list(
    profile: BearProfile,
    active_task_list: Option<&TaskListProjection>,
    assistant_text: &str,
) -> bool {
    let kind = classify_autonomous_final_response(assistant_text);
    autonomous_execution_gate_for_task_list(profile, active_task_list, kind).may_stop
}

pub fn autonomous_resume_obligation_text(plan: &WorkPlanProjection) -> Option<String> {
    if !matches!(plan.owner_profile.as_str(), "pair" | "work") {
        return None;
    }
    let items = plan
        .items
        .iter()
        .map(|item| {
            let marker = match item.status {
                WorkPlanItemStatus::Completed => "[x]",
                _ => "[ ]",
            };
            format!("- {marker} {}", item.title)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let next = next_incomplete_unblocked_item(&plan.items)
        .map(|item| item.title.as_str())
        .unwrap_or("resolve blocker before proceeding");
    Some(format!(
        "Previous assistant turn ended before completing an autonomous implementation task.\n\nActive goal:\n{}\n\nContinuation policy:\nContinue the next incomplete, unblocked task. Do not provide a progress-only final answer unless the work is complete, blocked, not applicable, waived, or permission-gated. If the next planned action is inappropriate, mark that item blocked or cancelled with evidence and report the reason.\n\nCurrent task state:\n{}\n\nRequired action:\nResume execution from the next incomplete actionable item: {}.",
        plan.title, items, next
    ))
}

fn workplan_domain_json(
    policy: &ResolvedSessionPolicy,
    workplan_row: Option<&plan_mode::PlanModeSessionRow>,
) -> Value {
    let state = workflow_state_label(policy);
    let approval_status =
        approval_status_label(policy.plan_mode_state.as_deref(), policy.mode_label);
    json!({
        "domain": "workplan",
        "state": state,
        "approval_status": approval_status,
        "plan_id": workplan_row.map(|row| Value::from(row.id.to_string())).unwrap_or(Value::Null),
        "id": workplan_row.map(|row| Value::from(row.id.to_string())).unwrap_or(Value::Null),
        "root_id": workplan_row.map(|row| Value::from(row.id.to_string())).unwrap_or(Value::Null),
        "parent_id": Value::Null,
        "relation": if state == "inactive" { "none" } else { "root" },
        "mode_label": policy.mode_label,
        "raw_state": workplan_row.map(|row| Value::from(row.state.clone())).unwrap_or(Value::Null),
        "title": workplan_row.and_then(|row| row.plan_title.clone()).map(Value::from).unwrap_or(Value::Null),
        "summary": workplan_row
            .and_then(|row| row.plan_body.as_ref().map(|body| summarize_text(body, 240)))
            .map(Value::from)
            .unwrap_or(Value::Null),
        "artifact_path": workplan_row
            .and_then(|row| row.plan_artifact_path.clone())
            .map(Value::from)
            .unwrap_or(Value::Null),
        "submitted_plan_present": workplan_row
            .map(|row| row.plan_artifact_path.is_some())
            .unwrap_or(false),
        "execution_unlocked": approval_status == "approved_execution_unlocked",
        "execution_unlocked_when_approved": policy.tool_enablement.enables_non_read_tools(),
        "approved_at": workplan_row
            .and_then(|row| row.approved_at.map(|t| Value::from(t.to_string())))
            .unwrap_or(Value::Null),
        "closed_at": workplan_row
            .and_then(|row| row.closed_at.map(|t| Value::from(t.to_string())))
            .unwrap_or(Value::Null),
        "updated_at": workplan_row
            .map(|row| Value::from(row.updated_at.to_string()))
            .unwrap_or(Value::Null),
    })
}

fn activity_domain_json(plan: Option<&WorkPlanProjection>) -> Value {
    match plan {
        Some(plan) => {
            let counts = activity_item_counts(plan);
            let status_sync_required =
                !matches!(plan.status.as_str(), "completed" | "cancelled" | "archived");
            json!({
                "domain": "activity",
                "plan_id": plan.id,
                "id": plan.id,
                "root_id": plan.id,
                "parent_id": Value::Null,
                "relation": "root",
                "frontmost": true,
                "status": plan.status,
                "title": plan.title,
                "summary": plan.summary,
                "current_item": plan.current_item.as_ref().map(activity_item_json).unwrap_or(Value::Null),
                "counts": counts,
                "status_sync_required": status_sync_required,
                "completion_claim_requires_status_update": status_sync_required,
                "status_update_tool": if status_sync_required { Value::from("update_task_list") } else { Value::Null },
                "toward_workplan_id": Value::Null,
                "handoff_requested": plan.handoff_intent_path.is_some() || plan.handoff_task_id.is_some(),
                "visibility": plan.visibility,
                "owner_profile": plan.owner_profile,
                "version": plan.version,
            })
        }
        None => json!({
            "domain": "activity",
            "plan_id": Value::Null,
            "id": Value::Null,
            "root_id": Value::Null,
            "parent_id": Value::Null,
            "relation": "none",
            "frontmost": false,
            "status": "inactive",
            "title": Value::Null,
            "summary": Value::Null,
            "current_item": Value::Null,
            "counts": {
                "pending": 0,
                "in_progress": 0,
                "blocked": 0,
                "completed": 0,
                "cancelled": 0
            },
            "status_sync_required": false,
            "completion_claim_requires_status_update": false,
            "status_update_tool": Value::Null,
            "toward_workplan_id": Value::Null,
            "handoff_requested": false
        }),
    }
}

fn autonomous_execution_domain_json(plan: Option<&WorkPlanProjection>) -> Value {
    let profile = plan
        .and_then(|plan| BearProfile::from_str(&plan.owner_profile).ok())
        .unwrap_or(BearProfile::Pair);
    let Some(plan) = plan.filter(|plan| is_autonomous_implementation_plan(profile, plan)) else {
        return json!({
            "mode": Value::Null,
            "active": false,
        });
    };
    let gate = autonomous_execution_gate_for_plan(
        profile,
        Some(plan),
        AutonomousFinalResponseKind::ProgressReport,
    );
    let last_verified_completed_step = plan
        .items
        .iter()
        .rev()
        .find(|item| item.status == WorkPlanItemStatus::Completed)
        .map(|item| Value::from(item.title.clone()))
        .unwrap_or(Value::Null);
    json!({
        "mode": "autonomous_implementation",
        "active": true,
        "goal": plan.title,
        "acceptance_criteria": plan.summary,
        "continuation_policy": AUTONOMOUS_CONTINUATION_POLICY,
        "stop_conditions": [
            "acceptance_criteria_met",
            "hard_blocker",
            "unsafe_or_external_action_required"
        ],
        "tasks": plan.items.iter().map(autonomous_task_json).collect::<Vec<_>>(),
        "current_in_progress_item": plan.current_item.as_ref().map(|item| item.title.clone()),
        "known_blockers": plan.items.iter().filter(|item| item.status == WorkPlanItemStatus::Blocked).filter_map(|item| item.blocked_reason.clone()).collect::<Vec<_>>(),
        "last_verified_completed_step": last_verified_completed_step,
        "has_incomplete_unblocked_items": gate.has_incomplete_unblocked_items,
        "next_incomplete_task_title": gate.next_incomplete_task_title,
    })
}

fn autonomous_task_json(item: &WorkPlanItem) -> Value {
    json!({
        "title": item.title,
        "status": item.status.as_str(),
        "evidence": item.summary,
        "blocked_reason": item.blocked_reason,
    })
}

fn is_autonomous_implementation_plan(profile: BearProfile, plan: &WorkPlanProjection) -> bool {
    matches!(profile, BearProfile::Pair | BearProfile::Work)
        && matches!(plan.owner_profile.as_str(), "pair" | "work")
        && matches!(plan.status.as_str(), "active" | "blocked" | "completed" | "cancelled")
}

fn is_autonomous_task_list(profile: BearProfile, task_list: &TaskListProjection) -> bool {
    matches!(profile, BearProfile::Pair | BearProfile::Work)
        && matches!(task_list.owner_profile.as_str(), "pair" | "work")
        && matches!(task_list.status.as_str(), "active" | "ready" | "running" | "blocked" | "completed" | "cancelled")
}

fn acceptance_criteria_met(plan: &WorkPlanProjection) -> bool {
    !plan.items.is_empty()
        && plan
            .items
            .iter()
            .all(|item| matches!(item.status, WorkPlanItemStatus::Completed | WorkPlanItemStatus::Cancelled))
        && matches!(plan.status.as_str(), "completed" | "cancelled")
}

fn next_incomplete_unblocked_item(items: &[WorkPlanItem]) -> Option<&WorkPlanItem> {
    items.iter().find(|item| {
        matches!(item.status, WorkPlanItemStatus::Pending | WorkPlanItemStatus::InProgress)
    })
}

fn task_list_acceptance_criteria_met(task_list: &TaskListProjection) -> bool {
    !task_list.items.is_empty()
        && task_list
            .items
            .iter()
            .all(|item| matches!(item.status, WorkPlanItemStatus::Completed | WorkPlanItemStatus::Cancelled))
        && matches!(task_list.status.as_str(), "completed" | "cancelled")
}

fn next_incomplete_unblocked_task_list_item(items: &[TaskListItem]) -> Option<&TaskListItem> {
    items.iter().find(|item| {
        matches!(item.status, WorkPlanItemStatus::Pending | WorkPlanItemStatus::InProgress)
    })
}

fn activity_item_json(item: &den_docket::WorkPlanItem) -> Value {
    json!({
        "id": item.id,
        "title": item.title,
        "summary": item.summary,
        "status": item.status.as_str(),
        "blocked_reason": item.blocked_reason,
        "source_refs": item.source_refs,
    })
}

fn activity_item_counts(plan: &WorkPlanProjection) -> Value {
    let mut pending = 0;
    let mut in_progress = 0;
    let mut blocked = 0;
    let mut completed = 0;
    let mut cancelled = 0;
    for item in &plan.items {
        match item.status.as_str() {
            "pending" => pending += 1,
            "in_progress" => in_progress += 1,
            "blocked" => blocked += 1,
            "completed" => completed += 1,
            "cancelled" => cancelled += 1,
            _ => {}
        }
    }
    json!({
        "pending": pending,
        "in_progress": in_progress,
        "blocked": blocked,
        "completed": completed,
        "cancelled": cancelled,
    })
}

fn memory_domain_json() -> Value {
    json!({
        "domain": "memory",
        "write_allowed": true,
        "active_plan_write_allowed": false,
        "write_for_active_workplan_allowed": false,
        "review_requested": false,
        "active_scope": "role-local"
    })
}

fn execution_domain_json(policy: &ResolvedSessionPolicy) -> Value {
    let execution_unlocked = policy.tool_enablement.enables_non_read_tools();
    json!({
        "domain": "execution",
        "permission_mode": policy.mode_label,
        "tool_classes": policy.allowed_tool_classes(),
        "execution_unlocked": execution_unlocked,
        "local_tools_available": true,
        "approval_required_for_mutation": execution_unlocked
    })
}

fn summarize_text(body: &str, max_chars: usize) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let mut summary = trimmed.chars().take(max_chars).collect::<String>();
        summary.push('…');
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use den_docket::{TaskListItem, TaskListProjection, TaskListSourceRef, TaskListSyncState, WorkPlanProjection};
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn item(title: &str, status: WorkPlanItemStatus) -> WorkPlanItem {
        WorkPlanItem {
            id: title.to_string(),
            title: title.to_string(),
            summary: Some(format!("evidence: {title}")),
            status,
            blocked_reason: (status == WorkPlanItemStatus::Blocked).then(|| "waiting".to_string()),
            source_refs: Vec::new(),
        }
    }

    fn plan(status: &str, items: Vec<WorkPlanItem>) -> WorkPlanProjection {
        WorkPlanProjection {
            id: Uuid::nil(),
            bear_id: Uuid::nil(),
            title: "Complete Docket relational work management".to_string(),
            summary: "Acceptance criteria".to_string(),
            owner_profile: "pair".to_string(),
            visibility: "bear_visible".to_string(),
            status: status.to_string(),
            version: 1,
            current_item: items
                .iter()
                .find(|item| item.status == WorkPlanItemStatus::InProgress)
                .cloned(),
            items,
            source_conversation_id: None,
            source_client_session_id: None,
            handoff_intent_path: None,
            handoff_task_id: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn task_list_item(title: &str, status: WorkPlanItemStatus) -> TaskListItem {
        TaskListItem {
            id: title.to_string(),
            title: title.to_string(),
            summary: Some(format!("evidence: {title}")),
            status,
            blocked_reason: (status == WorkPlanItemStatus::Blocked).then(|| "permission needed".to_string()),
            source_ref: TaskListSourceRef::local(Vec::new()),
            sync_state: TaskListSyncState::LocalOnly,
        }
    }

    fn task_list(status: &str, items: Vec<TaskListItem>) -> TaskListProjection {
        TaskListProjection {
            id: Uuid::nil(),
            bear_id: Uuid::nil(),
            title: "Implementation".to_string(),
            summary: "Acceptance criteria".to_string(),
            owner_profile: "pair".to_string(),
            visibility: "bear_visible".to_string(),
            status: status.to_string(),
            version: 1,
            source_ref: TaskListSourceRef::local(Vec::new()),
            current_item: items
                .iter()
                .find(|item| item.status == WorkPlanItemStatus::InProgress)
                .cloned(),
            items,
            source_conversation_id: None,
            source_client_session_id: None,
            handoff_intent_path: None,
            handoff_task_id: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn autonomous_resume_obligation_lists_next_incomplete_item() {
        let plan = plan(
            "active",
            vec![
                item("Inventory schema and Docket API coupling", WorkPlanItemStatus::Completed),
                item("Add lifecycle/dispatcher tests", WorkPlanItemStatus::Pending),
            ],
        );
        let text = autonomous_resume_obligation_text(&plan).expect("autonomous reminder");
        assert!(text.contains("Add lifecycle/dispatcher tests"));
        assert!(text.contains("Do not provide a progress-only final answer"));
    }

    #[test]
    fn autonomous_gate_blocks_progress_report_while_work_remains() {
        let plan = plan(
            "active",
            vec![
                item("done", WorkPlanItemStatus::Completed),
                item("remaining", WorkPlanItemStatus::InProgress),
            ],
        );
        let gate = autonomous_execution_gate_for_plan(
            BearProfile::Pair,
            Some(&plan),
            classify_autonomous_final_response(
                "What I changed: added one test. Remaining work: gate final answers.",
            ),
        );
        assert!(gate.is_active_autonomous_task);
        assert!(gate.has_incomplete_unblocked_items);
        assert!(!gate.may_stop);
    }

    #[test]
    fn autonomous_gate_allows_completion_only_when_plan_complete() {
        let plan = plan(
            "completed",
            vec![item("done", WorkPlanItemStatus::Completed)],
        );
        let gate = autonomous_execution_gate_for_plan(
            BearProfile::Pair,
            Some(&plan),
            AutonomousFinalResponseKind::CompletionFinal,
        );
        assert!(gate.acceptance_criteria_met);
        assert!(gate.may_stop);
    }

    #[test]
    fn autonomous_gate_allows_blocked_final_when_no_safe_path_remains() {
        let plan = plan(
            "blocked",
            vec![item("blocked", WorkPlanItemStatus::Blocked)],
        );
        let gate = autonomous_execution_gate_for_plan(
            BearProfile::Pair,
            Some(&plan),
            AutonomousFinalResponseKind::BlockedFinal,
        );
        assert!(gate.has_hard_blocker);
        assert!(gate.may_stop);
    }

    #[test]
    fn pair_without_active_task_list_does_not_trigger_terminal_gate() {
        assert!(should_allow_terminal_response(
            BearProfile::Pair,
            None,
            "What I changed: added one test. Remaining work: more later."
        ));
    }

    #[test]
    fn cancelled_remaining_task_allows_reasoned_non_action_final() {
        let task_list = task_list(
            "active",
            vec![
                task_list_item("Implement change", WorkPlanItemStatus::Completed),
                task_list_item("Commit changes", WorkPlanItemStatus::Cancelled),
            ],
        );

        let gate = autonomous_execution_gate_for_task_list(
            BearProfile::Pair,
            Some(&task_list),
            classify_autonomous_final_response(
                "I did not commit because there are no relevant changes to commit.",
            ),
        );

        assert!(gate.is_active_autonomous_task);
        assert!(!gate.has_incomplete_unblocked_items);
        assert!(gate.may_stop);
    }

    #[test]
    fn blocked_remaining_task_allows_blocker_final() {
        let task_list = task_list(
            "active",
            vec![
                task_list_item("Implement change", WorkPlanItemStatus::Completed),
                task_list_item("Commit changes", WorkPlanItemStatus::Blocked),
            ],
        );

        let gate = autonomous_execution_gate_for_task_list(
            BearProfile::Pair,
            Some(&task_list),
            classify_autonomous_final_response(
                "I am blocked because committing requires explicit permission.",
            ),
        );

        assert!(gate.has_hard_blocker);
        assert!(gate.may_stop);
    }
}
