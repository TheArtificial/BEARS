use crate::acp::stream::mapping::map_runtime_stream_event_to_acp_adapter_events_with_persistence;
use crate::acp::stream::support::AcpStreamDiagnostics;
use crate::acp::AcpStreamContext;
use den_runtime::tool_turns::ToolTurnCoordinator;
use den_runtime::gateway_events::GatewayEvent;
use den_runtime::role_runtime::{RoleRuntime, RoleTurnScope};
use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use uuid::Uuid;

fn test_mapping_context() -> AcpStreamContext {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
        .unwrap();
    let registry = ToolTurnCoordinator::new();
    let request_id = Uuid::new_v4();
    let role_runtime = RoleRuntime::new(registry.clone());
    let turn_scope = RoleTurnScope::acp_pair(
        Uuid::new_v4(),
        "acp-dedup-session",
        Some("den-conv-dedup".to_string()),
    );
    AcpStreamContext {
        pool,
        tool_turns: registry,
        user_id: 1,
        user_profile: None,
        bear_id: Uuid::new_v4(),
        bear_slug: "test-bear".to_string(),
        acp_session_id: "acp-dedup-session".to_string(),
        client: "zed".to_string(),
        conversation_id: "den-conv-dedup".to_string(),
        conversation_selection: "new-dedup".to_string(),
        resolved_conversation_id: Some("den-conv-dedup".to_string()),
        upstream_target: "den-conv-dedup".to_string(),
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: None,
        activity: None,
        request_id,
        pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
        config: Arc::new(den_core::config::Config::test_stub()),
        role_runtime,
        turn_scope,
        prompt_memory_diagnostic: serde_json::json!({}),
        memory_stores: den_runtime::memory::MemoryStoreManager::new(
            &den_core::config::Config::test_stub(),
        ),
    }
}

#[tokio::test]
async fn semantic_assistant_text_delta_emits_once_not_twice() {
    let context = test_mapping_context();
    let runtime_event = RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta {
        text: "hello".to_string(),
    });
    let mut diagnostics = AcpStreamDiagnostics::default();

    let (events, effect, adapter_result_rx) =
        map_runtime_stream_event_to_acp_adapter_events_with_persistence(
            runtime_event,
            context,
            &mut diagnostics,
        )
        .await
        .expect("mapping should succeed");

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        GatewayEvent::AssistantTextDelta { ref text } if text == "hello"
    ));
    assert!(effect.is_none());
    assert!(adapter_result_rx.is_none());
}

#[tokio::test]
async fn semantic_turn_completed_emits_single_turn_complete() {
    let context = test_mapping_context();
    let runtime_event =
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { turn: None });
    let mut diagnostics = AcpStreamDiagnostics::default();

    let (events, _, _) = map_runtime_stream_event_to_acp_adapter_events_with_persistence(
        runtime_event,
        context,
        &mut diagnostics,
    )
    .await
    .expect("mapping should succeed");

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        GatewayEvent::TurnComplete { ref outcome } if outcome == "ok"
    ));
}

#[tokio::test]
async fn requires_approval_stop_after_tool_request_is_not_unmapped() {
    let context = test_mapping_context();
    let mut diagnostics = AcpStreamDiagnostics::default();
    let tool_event = RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
        tool_call_id: "call-approval-1".to_string(),
        tool_name: "fs_read_text_file".to_string(),
        title: Some("Read text file".to_string()),
        kind: Some("read".to_string()),
        arguments: serde_json::json!({"path":"/workspace/README.md"}),
        approval_request_id: Some("approval-native-1".to_string()),
        approval_required: true,
        approval_reason: Some("workspace read".to_string()),
        run_id: None,
    });

    let (events, _, _) = map_runtime_stream_event_to_acp_adapter_events_with_persistence(
        tool_event,
        context.clone(),
        &mut diagnostics,
    )
    .await
    .expect("tool mapping should succeed");

    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], GatewayEvent::ToolRequest { .. }));
    assert!(diagnostics.unmapped_event_samples.is_empty());

    let pause_event = RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunPaused {
        reason: "requires_approval".to_string(),
        resume_token: Some("approval-native-1".to_string()),
        expires_at: None,
    });
    let (events, _, _) = map_runtime_stream_event_to_acp_adapter_events_with_persistence(
        pause_event,
        context,
        &mut diagnostics,
    )
    .await
    .expect("pause mapping should succeed");

    assert!(events.is_empty());
    assert!(diagnostics.unmapped_event_samples.is_empty());
    assert!(diagnostics.saw_requires_approval_stop);
}
