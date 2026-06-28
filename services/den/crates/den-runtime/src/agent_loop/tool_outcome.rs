//! Shared helpers for tool execution outcomes across web chat, ACP, and transcript repair.

use den_protocol::{RuntimeSemanticEvent, ToolCallFinishStatus};
use crate::{
    llm::ChatToolCall,
    
};

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

/// Whether a persisted or in-flight tool message body represents a failed execution.
pub fn tool_result_content_indicates_error(content: Option<&str>) -> bool {
    let Some(content) = content.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    if is_legacy_synthetic_interrupted_tool_result(Some(content))
        || is_incomplete_tool_result(Some(content))
    {
        return false;
    }
    let lower = content.to_ascii_lowercase();
    lower.starts_with("error:")
        || lower.starts_with("unsupported server tool:")
        || content.contains("\"ok\": false")
        || content.contains("\"ok\":false")
}

pub fn tool_message_counts_toward_llm_resolution(content: Option<&str>) -> bool {
    !is_incomplete_tool_result(content) && !is_legacy_synthetic_interrupted_tool_result(content)
}

pub fn tool_result_persistence_status(content: Option<&str>) -> &'static str {
    if is_incomplete_tool_result(content) {
        "incomplete"
    } else if tool_result_content_indicates_error(content) {
        "error"
    } else {
        "ok"
    }
}

pub fn user_visible_tool_summary(
    tool_name: &str,
    status: ToolCallFinishStatus,
    content: Option<&str>,
) -> String {
    match status {
        ToolCallFinishStatus::Ok => format!("Finished {tool_name}"),
        ToolCallFinishStatus::Incomplete => {
            format!("{tool_name} did not finish (turn interrupted).")
        }
        ToolCallFinishStatus::Cancelled => format!("{tool_name} was cancelled."),
        ToolCallFinishStatus::Error => user_visible_tool_error_summary(tool_name, content),
    }
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
