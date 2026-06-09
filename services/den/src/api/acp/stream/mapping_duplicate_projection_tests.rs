use crate::api::acp::stream::mapping::map_runtime_stream_event_to_acp_adapter_events_with_persistence;
use crate::api::acp::stream::support::AcpStreamDiagnostics;
use crate::api::acp::AcpStreamContext;
use crate::core::acp_tool_turns::AcpToolTurnCoordinator;
use crate::core::acp_letta_events::AcpGatewayEvent;
use crate::core::role_runtime::{RoleRuntime, RoleTurnScope};
use crate::core::runtime_provider::{RuntimeSemanticEvent, RuntimeStreamEvent};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use uuid::Uuid;

fn test_mapping_context() -> AcpStreamContext {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
        .unwrap();
    let registry = AcpToolTurnCoordinator::new();
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
        config: Arc::new(crate::config::Config::test_stub()),
        role_runtime,
        turn_scope,
        prompt_memory_diagnostic: serde_json::json!({}),
        memory_stores: crate::core::memory::MemoryStoreManager::new(
            &crate::config::Config::test_stub(),
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
        AcpGatewayEvent::AssistantTextDelta { ref text } if text == "hello"
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
        AcpGatewayEvent::TurnComplete { ref outcome } if outcome == "ok"
    ));
}
