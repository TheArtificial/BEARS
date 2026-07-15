//! Shared helpers for tool execution outcomes across web chat, client adapters, and transcript repair.

use crate::llm::ChatToolCall;
use den_core::tools::result_compaction::ToolResultStatus;
use den_protocol::{RuntimeSemanticEvent, ToolCallFinishStatus};

pub const LEGACY_SYNTHETIC_TOOL_RESULT_UNAVAILABLE: &str =
    "Tool result unavailable (prior turn interrupted).";

/// Internal marker stored on reconstructed tool messages with persisted `status: incomplete`.
pub const INCOMPLETE_TOOL_RESULT_MARK: &str = "__den_tool_result_incomplete__";

pub fn is_legacy_synthetic_interrupted_tool_result(content: Option<&str>) -> bool {
    content == Some(LEGACY_SYNTHETIC_TOOL_RESULT_UNAVAILABLE)
}

pub fn is_incomplete_tool_result(content: Option<&str>) -> bool {
    content == Some(INCOMPLETE_TOOL_RESULT_MARK)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolResultContentKind {
    Empty,
    LegacyInterrupted,
    Incomplete,
    Error,
    Ok,
}

impl ToolResultContentKind {
    pub const fn is_error(self) -> bool {
        matches!(self, Self::Error)
    }

    pub const fn counts_toward_llm_resolution(self) -> bool {
        !matches!(self, Self::Incomplete | Self::LegacyInterrupted)
    }
}

pub fn classify_tool_result_content(content: Option<&str>) -> ToolResultContentKind {
    let Some(content) = content.map(str::trim).filter(|s| !s.is_empty()) else {
        return ToolResultContentKind::Empty;
    };
    if is_legacy_synthetic_interrupted_tool_result(Some(content)) {
        return ToolResultContentKind::LegacyInterrupted;
    }
    if is_incomplete_tool_result(Some(content)) {
        return ToolResultContentKind::Incomplete;
    }
    let lower = content.to_ascii_lowercase();
    if lower.starts_with("error:")
        || lower.starts_with("unsupported server tool:")
        || content.contains("\"ok\": false")
        || content.contains("\"ok\":false")
    {
        ToolResultContentKind::Error
    } else {
        ToolResultContentKind::Ok
    }
}

/// Whether a persisted or in-flight tool message body represents a failed execution.
pub fn tool_result_content_indicates_error(content: Option<&str>) -> bool {
    classify_tool_result_content(content).is_error()
}

pub fn tool_message_counts_toward_llm_resolution(content: Option<&str>) -> bool {
    classify_tool_result_content(content).counts_toward_llm_resolution()
}

pub fn tool_result_persistence_status(content: Option<&str>) -> ToolResultStatus {
    match classify_tool_result_content(content) {
        ToolResultContentKind::Incomplete => ToolResultStatus::Incomplete,
        ToolResultContentKind::Error => ToolResultStatus::Error,
        ToolResultContentKind::Empty
        | ToolResultContentKind::LegacyInterrupted
        | ToolResultContentKind::Ok => ToolResultStatus::Ok,
    }
}

pub fn user_visible_tool_summary(
    tool_name: &str,
    status: ToolCallFinishStatus,
    content: Option<&str>,
) -> String {
    match status {
        ToolCallFinishStatus::Ok => docket_tool_summary(tool_name, content)
            .or_else(|| task_list_tool_summary(tool_name, content))
            .unwrap_or_else(|| format!("Finished {tool_name}")),
        ToolCallFinishStatus::Incomplete => {
            format!("{tool_name} did not finish (turn interrupted).")
        }
        ToolCallFinishStatus::Cancelled => format!("{tool_name} was cancelled."),
        ToolCallFinishStatus::Error => user_visible_tool_error_summary(tool_name, content),
    }
}

fn docket_tool_summary(tool_name: &str, content: Option<&str>) -> Option<String> {
    if !matches!(
        tool_name,
        "list_jobs"
            | "create_job"
            | "get_job"
            | "update_job"
            | "execute_job"
            | "list_tasks"
            | "create_task"
            | "update_task"
            | "update_current_task_status"
    ) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(content?).ok()?;
    match tool_name {
        "list_jobs" => summarize_docket_job_collection(&value),
        "create_job" => {
            docket_job_summary(&value).map(|summary| format!("Created Docket job: {summary}"))
        }
        "get_job" => {
            docket_job_summary(&value).map(|summary| format!("Read Docket job: {summary}"))
        }
        "update_job" => {
            docket_job_summary(&value).map(|summary| format!("Updated Docket job: {summary}"))
        }
        "execute_job" => {
            docket_job_summary(&value).map(|summary| format!("Executed Docket job: {summary}"))
        }
        "list_tasks" => summarize_docket_task_collection(&value),
        "create_task" => {
            docket_task_summary(&value).map(|summary| format!("Created Docket task: {summary}"))
        }
        "update_task" => {
            docket_task_summary(&value).map(|summary| format!("Updated Docket task: {summary}"))
        }
        "update_current_task_status" => docket_task_summary(&value)
            .map(|summary| format!("Updated Docket task status: {summary}")),
        _ => None,
    }
}

fn summarize_docket_job_collection(value: &serde_json::Value) -> Option<String> {
    let jobs = value
        .get("jobs")
        .or_else(|| value.get("items"))
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array())?;
    if jobs.is_empty() {
        return Some("Listed Docket jobs: none found.".to_string());
    }
    let mut goals = jobs
        .iter()
        .filter_map(docket_job_goal)
        .take(3)
        .collect::<Vec<_>>();
    let more = jobs.len().saturating_sub(goals.len());
    let goal_text = if goals.is_empty() {
        String::new()
    } else {
        if more > 0 {
            goals.push(format!("+{more} more"));
        }
        format!(": {}", goals.join(", "))
    };
    Some(format!(
        "Listed {} Docket job{}{}.",
        jobs.len(),
        if jobs.len() == 1 { "" } else { "s" },
        goal_text
    ))
}

fn summarize_docket_task_collection(value: &serde_json::Value) -> Option<String> {
    let tasks = value
        .get("tasks")
        .or_else(|| value.get("items"))
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array())?;
    if tasks.is_empty() {
        return Some("Listed Docket tasks: none found.".to_string());
    }
    let mut titles = tasks
        .iter()
        .filter_map(docket_task_title)
        .take(3)
        .collect::<Vec<_>>();
    let more = tasks.len().saturating_sub(titles.len());
    let title_text = if titles.is_empty() {
        String::new()
    } else {
        if more > 0 {
            titles.push(format!("+{more} more"));
        }
        format!(": {}", titles.join(", "))
    };
    Some(format!(
        "Listed {} Docket task{}{}.",
        tasks.len(),
        if tasks.len() == 1 { "" } else { "s" },
        title_text
    ))
}

fn docket_job_summary(value: &serde_json::Value) -> Option<String> {
    let goal = docket_job_goal(value)?;
    let job = value.get("job").unwrap_or(value);
    let mut parts = vec![format!("`{goal}`")];
    if let Some(status) = job.get("status").and_then(serde_json::Value::as_str) {
        parts.push(format!("status: {status}"));
    }
    if let Some(tasks) = value.get("tasks").and_then(serde_json::Value::as_array) {
        parts.push(format!(
            "{} task{}",
            tasks.len(),
            if tasks.len() == 1 { "" } else { "s" }
        ));
    }
    if let Some(criteria) = value.get("criteria").and_then(serde_json::Value::as_array) {
        parts.push(if criteria.len() == 1 {
            "1 criterion".to_string()
        } else {
            format!("{} criteria", criteria.len())
        });
    }
    if let Some(run) = value.get("current_run").filter(|run| !run.is_null()) {
        if let Some(state) = run.get("state").and_then(serde_json::Value::as_str) {
            parts.push(format!("run: {state}"));
        }
    }
    Some(parts.join(", "))
}

fn docket_job_goal(value: &serde_json::Value) -> Option<String> {
    value
        .get("job")
        .and_then(|job| job.get("goal"))
        .or_else(|| value.get("goal"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|goal| !goal.is_empty())
        .map(str::to_string)
}

fn docket_task_summary(value: &serde_json::Value) -> Option<String> {
    let title = docket_task_title(value)?;
    let task = value.get("task").unwrap_or(value);
    let mut parts = vec![format!("`{title}`")];
    if let Some(status) = value
        .get("run_state")
        .and_then(|state| state.get("status"))
        .or_else(|| task.get("status"))
        .and_then(serde_json::Value::as_str)
    {
        parts.push(format!("status: {status}"));
    }
    if let Some(role) = task
        .get("assigned_to_role")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|role| !role.is_empty())
    {
        parts.push(format!("assigned: {role}"));
    }
    Some(parts.join(", "))
}

fn docket_task_title(value: &serde_json::Value) -> Option<String> {
    value
        .get("task")
        .and_then(|task| task.get("title"))
        .or_else(|| value.get("title"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
}

fn task_list_tool_summary(tool_name: &str, content: Option<&str>) -> Option<String> {
    if !matches!(
        tool_name,
        "list_task_lists" | "get_task_list_status" | "update_task_list"
    ) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(content?).ok()?;
    match tool_name {
        "list_task_lists" => summarize_task_list_collection(&value),
        "get_task_list_status" => value
            .get("task_list")
            .filter(|task_list| !task_list.is_null())
            .map(|task_list| format!("Read task list status: {}", task_list_summary(task_list))),
        "update_task_list" => value
            .get("task_list")
            .filter(|task_list| !task_list.is_null())
            .map(|task_list| format!("Updated task list: {}", task_list_summary(task_list))),
        _ => None,
    }
}

fn summarize_task_list_collection(value: &serde_json::Value) -> Option<String> {
    let lists = value.get("task_lists")?.as_array()?;
    if lists.is_empty() {
        return Some("Listed task lists: none found.".to_string());
    }
    let mut titles = lists
        .iter()
        .filter_map(|list| list.get("title").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .take(3)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let more = lists.len().saturating_sub(titles.len());
    let title_text = if titles.is_empty() {
        String::new()
    } else {
        if more > 0 {
            titles.push(format!("+{more} more"));
        }
        format!(": {}", titles.join(", "))
    };
    Some(format!(
        "Listed {} task list{}{}.",
        lists.len(),
        if lists.len() == 1 { "" } else { "s" },
        title_text
    ))
}

fn task_list_summary(task_list: &serde_json::Value) -> String {
    let title = task_list
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Untitled task list");
    let items = task_list
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut pending = 0usize;
    let mut in_progress = 0usize;
    let mut blocked = 0usize;
    let mut completed = 0usize;
    for item in items {
        match item
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("pending")
        {
            "in_progress" => in_progress += 1,
            "blocked" => blocked += 1,
            "completed" => completed += 1,
            _ => pending += 1,
        }
    }
    let mut parts = vec![
        format!("`{title}`"),
        format!(
            "{} item{}",
            items.len(),
            if items.len() == 1 { "" } else { "s" }
        ),
    ];
    if in_progress > 0 {
        parts.push(format!("{in_progress} in progress"));
    }
    if pending > 0 {
        parts.push(format!("{pending} pending"));
    }
    if blocked > 0 {
        parts.push(format!("{blocked} blocked"));
    }
    if completed > 0 {
        parts.push(format!("{completed} completed"));
    }
    if let Some(current) = task_list
        .get("current_item")
        .and_then(|item| item.get("title"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        parts.push(format!("current: `{current}`"));
    }
    parts.join(", ")
}

pub fn user_visible_tool_error_summary(tool_name: &str, content: Option<&str>) -> String {
    if is_legacy_synthetic_interrupted_tool_result(content) {
        return format!("{tool_name} did not complete because the previous turn was interrupted.");
    }
    let Some(content) = content.map(str::trim).filter(|s| !s.is_empty()) else {
        return format!("{tool_name} failed.");
    };
    if let Some(rest) = content.strip_prefix("error:") {
        return format!("{tool_name} failed: {}", rest.trim());
    }
    if let Some(rest) = content.strip_prefix("unsupported server tool:") {
        return format!("{tool_name} is not available here: {}", rest.trim());
    }
    format!("{tool_name} failed: {content}")
}

pub fn tool_call_finished_event(
    call: &ChatToolCall,
    status: ToolCallFinishStatus,
    summary: impl Into<String>,
    error_message: Option<String>,
) -> RuntimeSemanticEvent {
    RuntimeSemanticEvent::ToolCallFinished {
        tool_call_id: call.id.clone(),
        tool_name: call.function.name.clone(),
        status,
        summary: Some(summary.into()),
        error_message,
    }
}

pub fn tool_call_finished_event_for_content(
    call: &ChatToolCall,
    content: Option<&str>,
) -> RuntimeSemanticEvent {
    let status = if tool_result_content_indicates_error(content) {
        ToolCallFinishStatus::Error
    } else {
        ToolCallFinishStatus::Ok
    };
    let summary = user_visible_tool_summary(&call.function.name, status, content);
    let error_message = if status == ToolCallFinishStatus::Error {
        Some(summary.clone())
    } else {
        None
    };
    tool_call_finished_event(call, status, summary, error_message)
}

pub fn tool_call_finished_event_for_incomplete(
    tool_call_id: impl Into<String>,
    tool_name: impl Into<String>,
    reason: &str,
) -> RuntimeSemanticEvent {
    let tool_name = tool_name.into();
    RuntimeSemanticEvent::ToolCallFinished {
        tool_call_id: tool_call_id.into(),
        tool_name: tool_name.clone(),
        status: ToolCallFinishStatus::Incomplete,
        summary: Some(format!("{tool_name} did not finish ({reason}).")),
        error_message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_list_list_summary_uses_returned_titles() {
        let content = serde_json::json!({
            "task_lists": [
                { "title": "Runtime fixes", "items": [] },
                { "title": "Docs", "items": [] }
            ]
        })
        .to_string();

        assert_eq!(
            user_visible_tool_summary("list_task_lists", ToolCallFinishStatus::Ok, Some(&content)),
            "Listed 2 task lists: Runtime fixes, Docs."
        );
    }

    #[test]
    fn task_list_status_summary_uses_counts_and_current_item() {
        let content = serde_json::json!({
            "task_list": {
                "title": "Runtime fixes",
                "items": [
                    { "title": "Trace bug", "status": "completed" },
                    { "title": "Patch code", "status": "in_progress" },
                    { "title": "Run tests", "status": "pending" }
                ],
                "current_item": { "title": "Patch code", "status": "in_progress" }
            }
        })
        .to_string();

        let summary = user_visible_tool_summary(
            "get_task_list_status",
            ToolCallFinishStatus::Ok,
            Some(&content),
        );

        assert!(
            summary.contains("Read task list status: `Runtime fixes`"),
            "{summary}"
        );
        assert!(summary.contains("3 items"), "{summary}");
        assert!(summary.contains("1 in progress"), "{summary}");
        assert!(summary.contains("1 pending"), "{summary}");
        assert!(summary.contains("1 completed"), "{summary}");
        assert!(summary.contains("current: `Patch code`"), "{summary}");
    }

    #[test]
    fn docket_job_summary_uses_goal_status_and_counts() {
        let content = serde_json::json!({
            "job": { "goal": "Improve Docket outputs", "status": "running" },
            "current_run": { "state": "running" },
            "tasks": [
                { "title": "Find seam" },
                { "title": "Patch summary" }
            ],
            "criteria": [{ "description": "Readable output" }]
        })
        .to_string();

        assert_eq!(
            user_visible_tool_summary("get_job", ToolCallFinishStatus::Ok, Some(&content)),
            "Read Docket job: `Improve Docket outputs`, status: running, 2 tasks, 1 criterion, run: running"
        );
    }

    #[test]
    fn docket_task_status_summary_uses_title_status_and_assignment() {
        let content = serde_json::json!({
            "task": { "title": "Patch summary", "assigned_to_role": "pair" },
            "run_state": { "status": "done" }
        })
        .to_string();

        assert_eq!(
            user_visible_tool_summary(
                "update_current_task_status",
                ToolCallFinishStatus::Ok,
                Some(&content)
            ),
            "Updated Docket task status: `Patch summary`, status: done, assigned: pair"
        );
    }
}
