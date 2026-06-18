use crate::acp::{
    tool_result_diagnostics::{
        delivered_tool_result_diagnostic, late_result_settlement_from_status,
        late_tool_result_ignored_diagnostic,
    },
    AcpToolResultResponse,
};
use den_runtime::{
    tool_turns::{ToolResultDelivery, ToolTurnCoordinator},
    client_tools::ToolStatus,
};

pub(super) fn default_unavailable_context_budget() -> serde_json::Value {
    serde_json::json!({
        "status": "unavailable",
        "reason": "Letta/provider context usage data is not wired into Den session_info yet",
        "source": "den.acp",
    })
}

pub(super) fn acp_tool_result_response_from_delivery(
    delivery: ToolResultDelivery,
    session_id: &str,
    tool_call_id_param: String,
    parsed_status: ToolStatus,
    tool_turns: &ToolTurnCoordinator,
) -> AcpToolResultResponse {
    match delivery {
        ToolResultDelivery::Delivered { body, .. } => AcpToolResultResponse {
            accepted: true,
            reason: "delivered".to_string(),
            settlement: None,
            turn_id: body.turn_id,
            tool_call_id: tool_call_id_param,
            diagnostic: Some(delivered_tool_result_diagnostic(parsed_status)),
        },
        ToolResultDelivery::TurnMissing {
            turn_id,
            tool_call_id,
        } => AcpToolResultResponse {
            accepted: false,
            reason: "late_result_ignored".to_string(),
            settlement: Some("unknown".to_string()),
            turn_id,
            tool_call_id,
            diagnostic: Some(late_tool_result_ignored_diagnostic()),
        },
        ToolResultDelivery::AlreadySettled {
            turn_id,
            tool_call_id,
        } => AcpToolResultResponse {
            accepted: true,
            reason: "duplicate_result_ignored".to_string(),
            settlement: Some("already_settled".to_string()),
            turn_id,
            tool_call_id: tool_call_id.clone(),
            diagnostic: tool_turns
                .recently_settled(session_id, &tool_call_id)
                .map(|cached| cached.diagnostic()),
        },
        ToolResultDelivery::RecentlySettled {
            turn_id,
            tool_call_id,
            cached,
        } => AcpToolResultResponse {
            accepted: false,
            reason: "late_result_ignored".to_string(),
            settlement: Some(late_result_settlement_from_status(&cached.status).to_string()),
            turn_id,
            tool_call_id,
            diagnostic: Some(cached.diagnostic()),
        },
    }
}
