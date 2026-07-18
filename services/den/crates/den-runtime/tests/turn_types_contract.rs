use den_runtime::{
    turn_ids::{
        ClientSessionId, PermissionId, ResponderRefId, SurfaceActionId, ToolCallId,
        TurnObligationId, TurnRunId, TurnStepId,
    },
    turn_obligations::{ExpectedResponderAction, TurnObligationKind, TurnObligationState},
    turn_runs::TurnRunState,
    turn_steps::TurnStepState,
};
use uuid::Uuid;

#[test]
fn typed_string_ids_reject_empty_values() {
    assert!(TurnRunId::new("run-1").is_ok());
    assert!(ClientSessionId::new("session-1").is_ok());
    assert!(ToolCallId::new("call-1").is_ok());
    assert!(PermissionId::new("perm-1").is_ok());
    assert!(ResponderRefId::new("slack-thread-1").is_ok());
    assert!(SurfaceActionId::new("action-1").is_ok());

    assert!(TurnRunId::new("").is_err());
    assert!(ClientSessionId::new("   ").is_err());
}

#[test]
fn typed_uuid_ids_display_and_round_trip() {
    let step = TurnStepId::new(Uuid::new_v4());
    assert_eq!(step.to_string(), step.as_uuid().to_string());

    let obligation = TurnObligationId::new(Uuid::new_v4());
    assert_eq!(obligation.to_string(), obligation.as_uuid().to_string());
}

#[test]
fn turn_state_parsers_accept_known_values_and_reject_unknown_values() {
    assert_eq!(
        TurnRunState::try_from_storage("waiting_for_client").unwrap(),
        TurnRunState::WaitingForClient
    );
    assert!(TurnRunState::try_from_storage("waiting_for_tool_result").is_err());
    assert!(TurnRunState::try_from_storage("waiting_for_permission").is_err());
    assert!(TurnRunState::try_from_storage("bearwire_waiting").is_err());

    assert_eq!(
        TurnStepState::try_from_storage("ready_to_continue").unwrap(),
        TurnStepState::ReadyToContinue
    );
    assert!(TurnStepState::try_from_storage("ready-ish").is_err());

    assert_eq!(
        TurnObligationState::try_from_storage("waiting_for_client").unwrap(),
        TurnObligationState::WaitingForClient
    );
    assert!(TurnObligationState::try_from_storage("waiting_for_bearwire").is_err());
}

#[test]
fn obligation_kind_and_expected_action_are_neutral() {
    assert_eq!(TurnObligationKind::ToolResult.as_str(), "tool_result");
    assert_eq!(
        TurnObligationKind::PermissionDecision.as_str(),
        "permission_decision"
    );
    assert_eq!(TurnObligationKind::HumanInput.as_str(), "human_input");
    assert_eq!(
        TurnObligationKind::ResourceBinding.as_str(),
        "resource_binding"
    );
    assert_eq!(
        TurnObligationKind::HandoffDecision.as_str(),
        "handoff_decision"
    );

    assert_eq!(ExpectedResponderAction::ToolResult.as_str(), "tool_result");
    assert_eq!(
        ExpectedResponderAction::PermissionDecision.as_str(),
        "permission_decision"
    );
    assert_eq!(ExpectedResponderAction::HumanInput.as_str(), "human_input");
    assert_eq!(
        ExpectedResponderAction::ResourceBinding.as_str(),
        "resource_binding"
    );
    assert_eq!(
        ExpectedResponderAction::HandoffDecision.as_str(),
        "handoff_decision"
    );

    assert!(ExpectedResponderAction::try_from_storage("client.tool.result").is_err());
    assert!(TurnObligationKind::try_from_storage("tool_call").is_err());
}
