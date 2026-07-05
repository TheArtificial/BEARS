
use super::*;

fn result_body(tool_call_id: Option<&str>) -> ToolResultRequest {
    ToolResultRequest {
        turn_id: Some("turn-1".to_string()),
        request_id: Some("request-1".to_string()),
        tool_call_id: tool_call_id.map(str::to_string),
        tool_name: Some("fs_read_text_file".to_string()),
        approval_request_id: Some("approval-1".to_string()),
        status: "ok".to_string(),
        content: Some("file contents".to_string()),
        structured_content: serde_json::json!({}),
        diagnostic: serde_json::json!({}),
        ..Default::default()
    }
}

#[test]
fn fills_missing_result_ids_from_registered_turn() {
    let coordinator = ToolTurnCoordinator::new();
    let (tx, mut rx) = oneshot::channel();
    coordinator
        .register(ToolTurnRegistration {
            user_id: 7,
            bear_id: Uuid::new_v4(),
            bear_slug: "meta".to_string(),
            client_session_id: "session-1".to_string(),
            request_id: Uuid::new_v4(),
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            approval_request_id: Some("approval-1".to_string()),
            timeout_ms: 30_000,
            result_tx: tx,
        })
        .unwrap();
    let delivery = coordinator
        .deliver_result(7, "meta", "session-1", "call-1", result_body(None))
        .unwrap();
    assert!(matches!(delivery, ToolResultDelivery::Delivered { .. }));
    let delivered = rx.try_recv().unwrap();
    assert_eq!(delivered.tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(delivered.approval_request_id.as_deref(), Some("approval-1"));
}

#[test]
fn registered_turn_is_visible_as_pending_for_session() {
    // The SSE stream keeps itself parked on outstanding obligations by reading
    // `pending_for_session`. Adapter-local tool requests must register here (they
    // previously did not), or the stream races to terminal and the result is rejected
    // as `late_result_ignored`.
    let coordinator = ToolTurnCoordinator::new();
    let request_id = Uuid::new_v4();
    let (tx, _rx) = oneshot::channel();
    coordinator
        .register(ToolTurnRegistration {
            user_id: 7,
            bear_id: Uuid::new_v4(),
            bear_slug: "meta".to_string(),
            client_session_id: "session-1".to_string(),
            request_id,
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            approval_request_id: Some("approval-1".to_string()),
            timeout_ms: 30_000,
            result_tx: tx,
        })
        .unwrap();
    let pending = coordinator.pending_for_session("session-1");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tool_call_id, "call-1");
    assert_eq!(pending[0].request_id, request_id);
}

#[test]
fn approval_request_id_mismatch_is_rejected() {
    // The registered obligation's approval id must match what the client echoes back.
    // A regenerated/dropped approval id surfaced here as a 400 (ValidationError).
    let coordinator = ToolTurnCoordinator::new();
    let (tx, _rx) = oneshot::channel();
    coordinator
        .register(ToolTurnRegistration {
            user_id: 7,
            bear_id: Uuid::new_v4(),
            bear_slug: "meta".to_string(),
            client_session_id: "session-1".to_string(),
            request_id: Uuid::new_v4(),
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            approval_request_id: Some("approval-expected".to_string()),
            timeout_ms: 30_000,
            result_tx: tx,
        })
        .unwrap();
    let mut body = result_body(Some("call-1"));
    body.approval_request_id = Some("approval-WRONG".to_string());
    let err = coordinator
        .deliver_result(7, "meta", "session-1", "call-1", body)
        .expect_err("mismatched approval id must be rejected");
    assert!(
        matches!(err, DenError::ValidationError(_)),
        "expected ValidationError, got {err:?}"
    );
}

#[test]
fn duplicate_after_removal_reports_recently_settled() {
    let coordinator = ToolTurnCoordinator::new();
    let (tx, _rx) = oneshot::channel();
    let request_id = Uuid::new_v4();
    coordinator
        .register(ToolTurnRegistration {
            user_id: 7,
            bear_id: Uuid::new_v4(),
            bear_slug: "meta".to_string(),
            client_session_id: "session-1".to_string(),
            request_id,
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            approval_request_id: Some("approval-1".to_string()),
            timeout_ms: 30_000,
            result_tx: tx,
        })
        .unwrap();
    assert!(matches!(
        coordinator
            .deliver_result(
                7,
                "meta",
                "session-1",
                "call-1",
                result_body(Some("call-1"))
            )
            .unwrap(),
        ToolResultDelivery::Delivered { .. }
    ));
    coordinator.remove("session-1", "call-1");
    match coordinator
        .deliver_result(
            7,
            "meta",
            "session-1",
            "call-1",
            result_body(Some("call-1")),
        )
        .unwrap()
    {
        ToolResultDelivery::RecentlySettled { cached, .. } => {
            assert_eq!(cached.request_id, request_id);
            assert_eq!(cached.tool_name, "fs_read_text_file");
            assert_eq!(cached.status, "ok");
            assert_eq!(cached.content_bytes, "file contents".len());
        }
        other => panic!("unexpected delivery: {other:?}"),
    }
}

#[test]
fn duplicate_result_reports_already_settled() {
    let coordinator = ToolTurnCoordinator::new();
    let (tx, _rx) = oneshot::channel();
    coordinator
        .register(ToolTurnRegistration {
            user_id: 7,
            bear_id: Uuid::new_v4(),
            bear_slug: "meta".to_string(),
            client_session_id: "session-1".to_string(),
            request_id: Uuid::new_v4(),
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            approval_request_id: Some("approval-1".to_string()),
            timeout_ms: 30_000,
            result_tx: tx,
        })
        .unwrap();
    assert!(matches!(
        coordinator
            .deliver_result(
                7,
                "meta",
                "session-1",
                "call-1",
                result_body(Some("call-1"))
            )
            .unwrap(),
        ToolResultDelivery::Delivered { .. }
    ));
    assert!(matches!(
        coordinator
            .deliver_result(
                7,
                "meta",
                "session-1",
                "call-1",
                result_body(Some("call-1"))
            )
            .unwrap(),
        ToolResultDelivery::AlreadySettled { .. }
    ));
}

#[test]
fn request_scoped_cleanup_preserves_other_request_and_active_turn() {
    let coordinator = ToolTurnCoordinator::new();
    let session_id = "session-1";
    let stale_request_id = Uuid::new_v4();
    let active_request_id = Uuid::new_v4();
    let _guard = coordinator
        .acquire_active_turn(session_id, active_request_id, Some("conv-1".to_string()))
        .unwrap();
    let (stale_tx, _stale_rx) = oneshot::channel();
    let (active_tx, _active_rx) = oneshot::channel();
    coordinator
        .register(ToolTurnRegistration {
            user_id: 7,
            bear_id: Uuid::new_v4(),
            bear_slug: "meta".to_string(),
            client_session_id: session_id.to_string(),
            request_id: stale_request_id,
            tool_call_id: "call-stale".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            approval_request_id: Some("approval-stale".to_string()),
            timeout_ms: 30_000,
            result_tx: stale_tx,
        })
        .unwrap();
    coordinator
        .register(ToolTurnRegistration {
            user_id: 7,
            bear_id: Uuid::new_v4(),
            bear_slug: "meta".to_string(),
            client_session_id: session_id.to_string(),
            request_id: active_request_id,
            tool_call_id: "call-active".to_string(),
            tool_name: "fs_edit_file".to_string(),
            approval_request_id: Some("approval-active".to_string()),
            timeout_ms: 30_000,
            result_tx: active_tx,
        })
        .unwrap();

    let summary = coordinator.cleanup_request_tool_turns(session_id, stale_request_id);

    assert_eq!(summary.pending_removed, 1);
    assert_eq!(summary.settled_removed, 0);
    let pending = coordinator.pending_for_session(session_id);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request_id, active_request_id);
    assert_eq!(pending[0].tool_call_id, "call-active");
    assert_eq!(
        coordinator
            .active_turn_for_session(session_id)
            .map(|turn| turn.request_id),
        Some(active_request_id)
    );
    assert!(matches!(
        coordinator
            .deliver_result(
                7,
                "meta",
                session_id,
                "call-active",
                ToolResultRequest {
                    tool_call_id: Some("call-active".to_string()),
                    tool_name: Some("fs_edit_file".to_string()),
                    approval_request_id: Some("approval-active".to_string()),
                    status: "ok".to_string(),
                    content: Some("edited".to_string()),
                    structured_content: serde_json::json!({}),
                    diagnostic: serde_json::json!({}),
                    ..Default::default()
                }
            )
            .unwrap(),
        ToolResultDelivery::Delivered { .. }
    ));
}

#[test]
fn expired_cleanup_preserves_nonexpired_request_and_active_turn() {
    let coordinator = ToolTurnCoordinator::new();
    let session_id = "session-1";
    let expired_request_id = Uuid::new_v4();
    let active_request_id = Uuid::new_v4();
    let _guard = coordinator
        .acquire_active_turn(session_id, active_request_id, Some("conv-1".to_string()))
        .unwrap();
    let (expired_tx, _expired_rx) = oneshot::channel();
    let (active_tx, _active_rx) = oneshot::channel();
    coordinator
        .register(ToolTurnRegistration {
            user_id: 7,
            bear_id: Uuid::new_v4(),
            bear_slug: "meta".to_string(),
            client_session_id: session_id.to_string(),
            request_id: expired_request_id,
            tool_call_id: "call-expired".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            approval_request_id: Some("approval-expired".to_string()),
            timeout_ms: 1,
            result_tx: expired_tx,
        })
        .unwrap();
    coordinator
        .register(ToolTurnRegistration {
            user_id: 7,
            bear_id: Uuid::new_v4(),
            bear_slug: "meta".to_string(),
            client_session_id: session_id.to_string(),
            request_id: active_request_id,
            tool_call_id: "call-active".to_string(),
            tool_name: "fs_edit_file".to_string(),
            approval_request_id: Some("approval-active".to_string()),
            timeout_ms: 30_000,
            result_tx: active_tx,
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));

    let summary = coordinator.cleanup_expired_tool_turns_for_session(session_id);

    assert_eq!(summary.pending_removed, 1);
    assert_eq!(summary.settled_removed, 0);
    let pending = coordinator.pending_for_session(session_id);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request_id, active_request_id);
    assert_eq!(pending[0].tool_call_id, "call-active");
    assert_eq!(
        coordinator
            .active_turn_for_session(session_id)
            .map(|turn| turn.request_id),
        Some(active_request_id)
    );
}
