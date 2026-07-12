use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::turn_obligations::TurnObligationRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnSurfaceKind {
    BearWireArmature,
    WebChat,
    Slack,
    MacosApp,
}

impl TurnSurfaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BearWireArmature => "bearwire_armature",
            Self::WebChat => "web_chat",
            Self::Slack => "slack",
            Self::MacosApp => "macos_app",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceActionKind {
    BearWireClientToolResult,
    BearWireClientPermissionResult,
    ChatReply,
    WebApprovalDecision,
    SlackThreadReply,
    SlackApprovalDecision,
    MacosApprovalDecision,
    MacosResourceBinding,
    Unsupported,
}

impl SurfaceActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BearWireClientToolResult => "bearwire.client.tool.result",
            Self::BearWireClientPermissionResult => "bearwire.client.permission.result",
            Self::ChatReply => "chat.reply",
            Self::WebApprovalDecision => "web.approval.decision",
            Self::SlackThreadReply => "slack.thread.reply",
            Self::SlackApprovalDecision => "slack.approval.decision",
            Self::MacosApprovalDecision => "macos.approval.decision",
            Self::MacosResourceBinding => "macos.resource.binding",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceProjection {
    pub surface: TurnSurfaceKind,
    pub obligation_id: String,
    pub obligation_kind: String,
    pub expected_responder_action: String,
    pub action_kind: SurfaceActionKind,
    pub responder_ref_id: Option<String>,
    pub payload: Value,
}

impl SurfaceProjection {
    pub fn is_supported(&self) -> bool {
        self.action_kind != SurfaceActionKind::Unsupported
    }
}

pub fn project_obligation_for_surface(
    obligation: &TurnObligationRow,
    surface: TurnSurfaceKind,
) -> SurfaceProjection {
    let action_kind = match (surface, obligation.expected_responder_action.as_str()) {
        (TurnSurfaceKind::BearWireArmature, "tool_result") => {
            SurfaceActionKind::BearWireClientToolResult
        }
        (TurnSurfaceKind::BearWireArmature, "permission_decision") => {
            SurfaceActionKind::BearWireClientPermissionResult
        }
        (TurnSurfaceKind::WebChat, "human_input") => SurfaceActionKind::ChatReply,
        (TurnSurfaceKind::WebChat, "permission_decision") => SurfaceActionKind::WebApprovalDecision,
        (TurnSurfaceKind::Slack, "human_input") => SurfaceActionKind::SlackThreadReply,
        (TurnSurfaceKind::Slack, "permission_decision") => SurfaceActionKind::SlackApprovalDecision,
        (TurnSurfaceKind::MacosApp, "permission_decision") => {
            SurfaceActionKind::MacosApprovalDecision
        }
        (TurnSurfaceKind::MacosApp, "resource_binding") => SurfaceActionKind::MacosResourceBinding,
        _ => SurfaceActionKind::Unsupported,
    };

    SurfaceProjection {
        surface,
        obligation_id: obligation.id.to_string(),
        obligation_kind: obligation.kind.clone(),
        expected_responder_action: obligation.expected_responder_action.clone(),
        action_kind,
        responder_ref_id: obligation.responder_ref_id.clone(),
        payload: json!({
            "run_id": obligation.run_id,
            "session_id": obligation.session_id,
            "turn_step_id": obligation.turn_step_id.map(|id| id.to_string()),
            "tool_call_id": obligation.tool_call_id,
            "permission_id": obligation.permission_id,
            "responder_ref_id": obligation.responder_ref_id,
            "request": obligation.request_payload,
        }),
    }
}

pub fn bearwire_client_method_for_action(action: &str) -> Option<&'static str> {
    match action {
        "tool_result" => Some("client.tool.result"),
        "permission_decision" => Some("client.permission.result"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn_obligations::TurnObligationRow;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn obligation(kind: &str, action: &str) -> TurnObligationRow {
        TurnObligationRow {
            id: Uuid::new_v4(),
            run_id: "run-test".to_string(),
            session_id: "session-test".to_string(),
            kind: kind.to_string(),
            expected_responder_action: action.to_string(),
            tool_call_id: Some("call-test".to_string()),
            permission_id: Some("perm-test".to_string()),
            responder_ref_id: Some("ref-test".to_string()),
            state: "waiting_for_client".to_string(),
            turn_step_id: Some(Uuid::new_v4()),
            request_payload: serde_json::json!({"prompt":"hello"}),
            result_payload: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            completed_at: None,
        }
    }

    #[test]
    fn bearwire_projection_maps_tool_and_permission_actions() {
        let tool = project_obligation_for_surface(
            &obligation("tool_result", "tool_result"),
            TurnSurfaceKind::BearWireArmature,
        );
        assert_eq!(
            tool.action_kind,
            SurfaceActionKind::BearWireClientToolResult
        );
        assert!(tool.is_supported());

        let permission = project_obligation_for_surface(
            &obligation("permission_decision", "permission_decision"),
            TurnSurfaceKind::BearWireArmature,
        );
        assert_eq!(
            permission.action_kind,
            SurfaceActionKind::BearWireClientPermissionResult
        );
        assert_eq!(
            bearwire_client_method_for_action("permission_decision"),
            Some("client.permission.result")
        );
    }

    #[test]
    fn channel_projection_maps_human_input_and_approval_actions() {
        let web = project_obligation_for_surface(
            &obligation("human_input", "human_input"),
            TurnSurfaceKind::WebChat,
        );
        assert_eq!(web.action_kind, SurfaceActionKind::ChatReply);

        let slack = project_obligation_for_surface(
            &obligation("permission_decision", "permission_decision"),
            TurnSurfaceKind::Slack,
        );
        assert_eq!(slack.action_kind, SurfaceActionKind::SlackApprovalDecision);

        let macos = project_obligation_for_surface(
            &obligation("resource_binding", "resource_binding"),
            TurnSurfaceKind::MacosApp,
        );
        assert_eq!(macos.action_kind, SurfaceActionKind::MacosResourceBinding);
    }

    #[test]
    fn unsupported_surface_obligation_pairs_are_explicit() {
        let projected = project_obligation_for_surface(
            &obligation("resource_binding", "resource_binding"),
            TurnSurfaceKind::Slack,
        );
        assert_eq!(projected.action_kind, SurfaceActionKind::Unsupported);
        assert!(!projected.is_supported());
    }
}
