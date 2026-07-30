use den_runtime::{
    surface_projection::{project_obligation_for_surface, SurfaceActionKind, TurnSurfaceKind},
    turn_obligations::TurnObligationRow,
};
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
        lease_attempt_token_hash: None,
        claimed_at: None,
        lease_expires_at: None,
    }
}

#[test]
fn projects_bearwire_tool_and_permission_actions() {
    let tool_obligation = obligation("tool_result", "tool_result");
    let tool = project_obligation_for_surface(&tool_obligation, TurnSurfaceKind::BearWireArmature);
    assert_eq!(
        tool.action_kind,
        SurfaceActionKind::BearWireClientToolResult
    );
    assert!(tool.is_supported());
    assert_eq!(tool.obligation_id, tool_obligation.id.to_string());
    assert_eq!(tool.payload["tool_call_id"], "call-test");
    assert_eq!(
        tool.payload["turn_step_id"],
        tool_obligation.turn_step_id.unwrap().to_string()
    );

    let permission_obligation = obligation("permission_decision", "permission_decision");
    let permission =
        project_obligation_for_surface(&permission_obligation, TurnSurfaceKind::BearWireArmature);
    assert_eq!(
        permission.action_kind,
        SurfaceActionKind::BearWireClientPermissionResult
    );
    assert_eq!(
        permission.obligation_id,
        permission_obligation.id.to_string()
    );
    assert_eq!(permission.payload["permission_id"], "perm-test");
    assert_eq!(
        permission.payload["turn_step_id"],
        permission_obligation.turn_step_id.unwrap().to_string()
    );
}

#[test]
fn projects_channel_human_input_and_approval_actions() {
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
