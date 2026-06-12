//! Shared helpers for tool execution outcomes across web chat, ACP, and transcript repair.

pub const LEGACY_SYNTHETIC_TOOL_RESULT_UNAVAILABLE: &str =
    "Tool result unavailable (prior turn interrupted).";

pub fn is_legacy_synthetic_interrupted_tool_result(content: Option<&str>) -> bool {
    content == Some(LEGACY_SYNTHETIC_TOOL_RESULT_UNAVAILABLE)
}

/// Whether a persisted or in-flight tool message body represents a failed execution.
pub fn tool_result_content_indicates_error(content: Option<&str>) -> bool {
    let Some(content) = content.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    if is_legacy_synthetic_interrupted_tool_result(Some(content)) {
        return true;
    }
    let lower = content.to_ascii_lowercase();
    lower.starts_with("error:")
        || lower.starts_with("unsupported server tool:")
        || content.contains("\"ok\": false")
        || content.contains("\"ok\":false")
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

pub fn tool_result_persistence_status(content: Option<&str>) -> &'static str {
    if tool_result_content_indicates_error(content) {
        "error"
    } else {
        "ok"
    }
}
