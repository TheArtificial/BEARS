    use super::*;

    #[test]
    fn active_turn_cancel_registry_signals_and_unregisters_session_turn() {
        let registry = ActiveTurnCancelRegistry::new();
        let request_id = Uuid::new_v4();
        let (handle, cancel_rx) =
            registry.register("acp-session-1", request_id, Some("conv-1".to_string()));

        let active = registry
            .active_for_session("acp-session-1")
            .expect("active registration");
        assert_eq!(active.request_id, request_id);
        assert_eq!(active.conversation_id.as_deref(), Some("conv-1"));
        assert!(active.run_ids.is_empty());
        assert!(!*cancel_rx.borrow());

        let cancelled = registry
            .cancel_session("acp-session-1")
            .expect("cancelled registration");
        assert_eq!(cancelled.request_id, request_id);
        assert!(cancelled.run_ids.is_empty());
        assert!(*cancel_rx.borrow());

        drop(handle);
        assert!(registry.active_for_session("acp-session-1").is_none());
    }

    #[test]
    fn active_turn_cancel_registry_records_run_ids_for_matching_turn() {
        let registry = ActiveTurnCancelRegistry::new();
        let request_id = Uuid::new_v4();
        let wrong_request_id = Uuid::new_v4();
        let (_handle, _rx) = registry.register("acp-session-1", request_id, None);

        assert!(!registry.record_run_id("acp-session-1", request_id, "   "));
        assert!(!registry.record_run_id("acp-session-1", wrong_request_id, "run-wrong"));
        assert!(!registry.record_run_id("missing-session", request_id, "run-missing"));
        assert!(registry.record_run_id("acp-session-1", request_id, " run-1 "));
        assert!(!registry.record_run_id("acp-session-1", request_id, "run-1"));
        assert!(registry.record_run_id("acp-session-1", request_id, "run-2"));

        let active = registry
            .active_for_session("acp-session-1")
            .expect("active registration");
        assert_eq!(
            active.run_ids,
            vec!["run-1".to_string(), "run-2".to_string()]
        );

        let cancelled = registry
            .cancel_session("acp-session-1")
            .expect("cancelled registration");
        assert_eq!(
            cancelled.run_ids,
            vec!["run-1".to_string(), "run-2".to_string()]
        );
    }

    #[test]
    fn active_turn_cancel_registry_does_not_unregister_newer_turn_from_old_handle() {
        let registry = ActiveTurnCancelRegistry::new();
        let old_request_id = Uuid::new_v4();
        let new_request_id = Uuid::new_v4();
        let (old_handle, _old_rx) = registry.register("acp-session-1", old_request_id, None);
        let (_new_handle, _new_rx) = registry.register("acp-session-1", new_request_id, None);

        drop(old_handle);
        assert_eq!(
            registry
                .active_for_session("acp-session-1")
                .expect("newer turn survives")
                .request_id,
            new_request_id
        );
    }

    #[test]
    fn active_turn_runtime_snapshot_reports_idle_without_active_turn() {
        let registry = ActiveTurnCancelRegistry::new();
        let tool_turns = ToolTurnCoordinator::new();
        let snapshot = registry.runtime_snapshot_for_session("acp-session", &tool_turns);

        assert_eq!(snapshot["state"], "idle");
        assert_eq!(snapshot["active_turn"]["present"], false);
        assert_eq!(snapshot["active_turn"]["pending_obligations"], 0);
        assert_eq!(snapshot["active_turn"]["run_ids"], json!([]));
        assert_eq!(snapshot["source"], "acp_active_turn_registry");
    }

    #[test]
    fn active_turn_runtime_snapshot_reports_running_without_pending_tools() {
        let registry = ActiveTurnCancelRegistry::new();
        let tool_turns = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let (_handle, _rx) =
            registry.register("acp-session", request_id, Some("conv-test".to_string()));
        assert!(registry.record_run_id("acp-session", request_id, "run-snapshot"));
        let snapshot = registry.runtime_snapshot_for_session("acp-session", &tool_turns);

        assert_eq!(snapshot["state"], "running");
        assert_eq!(snapshot["active_turn"]["present"], true);
        assert_eq!(snapshot["active_turn"]["phase"], "Streaming");
        assert_eq!(snapshot["active_turn"]["request_id"], json!(request_id));
        assert_eq!(snapshot["active_turn"]["conversation_id"], "conv-test");
        assert_eq!(snapshot["active_turn"]["run_ids"], json!(["run-snapshot"]));
        assert_eq!(snapshot["active_turn"]["pending_obligations"], 0);
    }

    #[test]
    fn active_turn_runtime_snapshot_reports_requires_action_with_pending_tool() {
        let registry = ActiveTurnCancelRegistry::new();
        let tool_turns = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let (_handle, _rx) = registry.register("acp-session", request_id, None);
        let (tx, _rx) = tokio::sync::oneshot::channel();
        tool_turns
            .register(crate::tool_turns::ToolTurnRegistration {
                user_id: 1,
                bear_id: Uuid::new_v4(),
                bear_slug: "test-bear".to_string(),
                acp_session_id: "acp-session".to_string(),
                request_id,
                tool_call_id: "call-1".to_string(),
                tool_name: "fs_read_text_file".to_string(),
                approval_request_id: Some("approval-1".to_string()),
                timeout_ms: 30_000,
                result_tx: tx,
            })
            .unwrap();
        let snapshot = registry.runtime_snapshot_for_session("acp-session", &tool_turns);

        assert_eq!(snapshot["state"], "requires_action");
        assert_eq!(snapshot["active_turn"]["present"], true);
        assert_eq!(snapshot["active_turn"]["phase"], "WaitingForObligations");
        assert_eq!(snapshot["active_turn"]["pending_obligations"], 1);
        assert_eq!(snapshot["active_turn"]["pending_adapter_tools"], 1);
        assert_eq!(snapshot["active_turn"]["pending_den_tools"], 0);
    }

    #[test]
    fn acp_turn_text_only_completes_once() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        turn.on_stream_end();

        let terminal = turn.take_terminal_event().expect("terminal ready");
        assert_eq!(terminal.status, TerminalStatus::Ok);
        assert_eq!(terminal.reason, TerminalReason::EndTurn);
        assert_eq!(turn.take_terminal_event(), None);
        assert_eq!(turn.phase(), TurnPhase::Terminal);
    }

    #[test]
    fn acp_turn_waits_for_adapter_local_tool_before_terminal() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        turn.on_tool_request(
            "call_1",
            "fs_read_text_file",
            ToolExecutionRoute::AdapterLocal,
        );
        turn.on_requires_approval_stop();
        turn.on_stream_end();

        assert_eq!(turn.open_obligation_count(), 1);
        assert!(!turn.may_emit_terminal());
        assert_eq!(turn.take_terminal_event(), None);

        assert_eq!(
            turn.on_adapter_tool_result("call_1", true),
            ToolResultDisposition::Accepted
        );
        assert_eq!(turn.open_obligation_count(), 0);
        assert!(!turn.may_emit_terminal());

        turn.on_stream_started();
        turn.on_stream_end();
        let terminal = turn.take_terminal_event().expect("terminal ready");
        assert_eq!(terminal.status, TerminalStatus::Ok);
        assert_eq!(turn.take_terminal_event(), None);
    }

    #[test]
    fn acp_turn_den_server_tool_does_not_create_adapter_obligation() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        turn.on_tool_request("call_1", "session_info", ToolExecutionRoute::DenServer);

        let obligation = turn.obligation("call_1").expect("tracked Den obligation");
        assert_eq!(obligation.route, ToolExecutionRoute::DenServer);
        assert_eq!(obligation.status, ObligationStatus::Running);
        assert_eq!(turn.open_obligation_count(), 1);

        assert_eq!(
            turn.on_den_tool_settled("call_1", true),
            ToolResultDisposition::Accepted
        );
        assert_eq!(turn.open_obligation_count(), 0);
        turn.on_stream_end();
        assert_eq!(
            turn.take_terminal_event().expect("terminal ready").status,
            TerminalStatus::Ok
        );
    }

    #[test]
    fn acp_turn_unsupported_tool_settles_without_hanging() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        turn.on_tool_request("call_1", "unknown_tool", ToolExecutionRoute::Unsupported);

        assert_eq!(turn.open_obligation_count(), 0);
        let terminal = turn.take_terminal_event().expect("terminal ready");
        assert_eq!(terminal.status, TerminalStatus::Failed);
        assert_eq!(terminal.reason, TerminalReason::UnsupportedTool);
    }

    #[test]
    fn acp_turn_timeout_settles_pending_adapter_tool() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        turn.on_tool_request(
            "call_1",
            "fs_read_text_file",
            ToolExecutionRoute::AdapterLocal,
        );

        assert_eq!(
            turn.on_tool_timeout("call_1"),
            ToolResultDisposition::Accepted
        );
        assert_eq!(turn.open_obligation_count(), 0);
        let terminal = turn.take_terminal_event().expect("terminal ready");
        assert_eq!(terminal.status, TerminalStatus::Failed);
        assert_eq!(terminal.reason, TerminalReason::ToolTimeout);
        assert_eq!(
            turn.on_adapter_tool_result("call_1", true),
            ToolResultDisposition::LateIgnored
        );
    }

    #[test]
    fn acp_turn_cancel_settles_pending_adapter_tool() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        turn.on_tool_request(
            "call_1",
            "fs_read_text_file",
            ToolExecutionRoute::AdapterLocal,
        );
        turn.on_cancel();

        assert_eq!(turn.open_obligation_count(), 0);
        let obligation = turn.obligation("call_1").expect("obligation");
        assert_eq!(obligation.status, ObligationStatus::Cancelled);
        let terminal = turn.take_terminal_event().expect("terminal ready");
        assert_eq!(terminal.status, TerminalStatus::Cancelled);
        assert_eq!(terminal.reason, TerminalReason::Cancelled);
        assert_eq!(
            turn.on_adapter_tool_result("call_1", true),
            ToolResultDisposition::LateIgnored
        );
    }

    #[test]
    fn acp_turn_late_result_after_terminal_is_ignored() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        turn.on_stream_end();
        assert!(turn.take_terminal_event().is_some());

        assert_eq!(
            turn.on_adapter_tool_result("call_1", true),
            ToolResultDisposition::LateIgnored
        );
        assert_eq!(turn.late_results_ignored(), 1);
        assert_eq!(turn.take_terminal_event(), None);
    }

    #[test]
    fn acp_turn_orphaned_requires_approval_triggers_recovery_path() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        turn.on_requires_approval_stop();

        assert!(turn.orphaned_requires_approval());
        assert_eq!(
            turn.take_status_update().expect("status update").key,
            "recovering_stale_approval"
        );
        let terminal = turn.take_terminal_event().expect("terminal ready");
        assert_eq!(terminal.status, TerminalStatus::Recovered);
        assert_eq!(terminal.reason, TerminalReason::OrphanedRequiresApproval);
        assert_eq!(turn.take_terminal_event(), None);
    }

    #[test]
    fn acp_turn_status_snapshot_reports_phase_and_obligations() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        turn.on_tool_request(
            "call_local",
            "fs_read_text_file",
            ToolExecutionRoute::AdapterLocal,
        );
        turn.on_tool_request("call_den", "session_info", ToolExecutionRoute::DenServer);

        let snapshot = turn.status_snapshot();
        assert_eq!(snapshot.phase, TurnPhase::WaitingForObligations);
        assert_eq!(snapshot.open_obligations, 2);
        assert_eq!(snapshot.pending_adapter_tools, 1);
        assert_eq!(snapshot.pending_den_tools, 1);
        assert_eq!(snapshot.pending_permissions, 0);
        assert_eq!(snapshot.terminal_status, None);
        assert_eq!(snapshot.terminal_reason, None);

        assert_eq!(
            turn.on_adapter_tool_result("call_local", true),
            ToolResultDisposition::Accepted
        );
        assert_eq!(
            turn.on_den_tool_settled("call_den", true),
            ToolResultDisposition::Accepted
        );
        turn.on_stream_end();
        assert!(turn.take_terminal_event().is_some());

        let snapshot = turn.status_snapshot();
        assert_eq!(snapshot.phase, TurnPhase::Terminal);
        assert_eq!(snapshot.open_obligations, 0);
        assert_eq!(snapshot.pending_adapter_tools, 0);
        assert_eq!(snapshot.pending_den_tools, 0);
        assert_eq!(snapshot.terminal_status, Some(TerminalStatus::Ok));
        assert_eq!(snapshot.terminal_reason, Some(TerminalReason::EndTurn));
    }

    #[test]
    fn acp_turn_heartbeat_status_rotates_while_streaming() {
        let mut turn = TurnController::new();
        turn.on_stream_started();
        let first = turn.heartbeat_status_update();
        let second = turn.heartbeat_status_update();
        assert_ne!(first.text, second.text);
        assert!(first.text.contains("Connecting") || first.text.contains("Waiting"));
    }

    #[test]
    fn acp_turn_status_updates_are_deduplicated() {
        let mut turn = TurnController::new();
        assert_eq!(turn.take_status_update(), None);

        turn.on_stream_started();
        let planning = turn.take_status_update().expect("planning");
        assert_eq!(planning.key, "planning");
        assert!(planning.text.contains("Planning next step"));
        assert_eq!(turn.take_status_update(), None);

        turn.set_client_label("zed");
        turn.on_tool_request(
            "call_1",
            "fs_read_text_file",
            ToolExecutionRoute::AdapterLocal,
        );
        let waiting = turn.take_status_update().expect("waiting");
        assert_eq!(waiting.key, "waiting_local:fs_read_text_file");
        assert!(waiting.text.contains("Zed"));
        assert!(waiting.text.contains("Read file"));
        assert_eq!(turn.take_status_update(), None);

        assert_eq!(
            turn.on_adapter_tool_result("call_1", true),
            ToolResultDisposition::Accepted
        );
        let continuing = turn.take_status_update().expect("continuing");
        assert_eq!(continuing.key, "continuing_after:fs_read_text_file");
        assert!(continuing.text.contains("Read file"));
        assert_eq!(turn.take_status_update(), None);
    }
