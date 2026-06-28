use den_core::client_tools::{diag_phase, ToolStatus};

pub(super) fn delivered_tool_result_diagnostic(parsed_status: ToolStatus) -> serde_json::Value {
    serde_json::json!({
        "component": "den.acp",
        "phase": diag_phase::DEN_RESULT_DELIVERED,
        "status": parsed_status.as_str(),
    })
}

pub(super) fn late_tool_result_ignored_diagnostic() -> serde_json::Value {
    serde_json::json!({
        "component": "den.acp",
        "phase": "late_tool_result_ignored",
    })
}

pub(super) fn late_result_settlement_from_status(status: &str) -> &'static str {
    match status {
        "timeout" => "timed_out",
        "cancelled" => "cancelled",
        "ok" | "error" | "unsupported" => "already_settled",
        _ => "unknown",
    }
}
