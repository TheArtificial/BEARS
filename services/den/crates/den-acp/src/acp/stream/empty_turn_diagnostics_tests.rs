use crate::acp::{AcpGatewayEvent, AcpStreamContext};
use den_runtime::{
    runtime_provider::{RuntimeSemanticEvent, RuntimeStreamEvent},
    role_runtime::{RoleRuntime, RoleTurnScope},
};
use uuid::Uuid;

use super::support::AcpStreamDiagnostics;

fn test_context() -> AcpStreamContext {
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;

    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
        .unwrap();
    let registry = den_runtime::acp_tool_turns::AcpToolTurnCoordinator::new();
    let request_id = Uuid::new_v4();
    let role_runtime = RoleRuntime::new(registry.clone());
    let turn_scope = RoleTurnScope::acp_pair(
        Uuid::new_v4(),
        "acp-empty-turn-session",
        Some("conv-empty-turn".to_string()),
    );
    AcpStreamContext {
        pool,
        tool_turns: registry,
        user_id: 1,
        user_profile: None,
        bear_id: Uuid::new_v4(),
        bear_slug: "test-bear".to_string(),
        acp_session_id: "acp-empty-turn-session".to_string(),
        client: "zed".to_string(),
        conversation_id: "conv-empty-turn".to_string(),
        conversation_selection: "conv-empty-turn".to_string(),
        resolved_conversation_id: Some("conv-empty-turn".to_string()),
        upstream_target: "conv-empty-turn".to_string(),
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
async fn turn_controller_planning_status_does_not_suppress_empty_turn_error() {
    let mut diagnostics = AcpStreamDiagnostics::default();
    diagnostics.observe_mapped_event(
        &AcpGatewayEvent::StatusText {
            text: "Planning next step — may call Den memory tools or Zed workspace tools…"
                .to_string(),
        },
        false,
    );
    diagnostics.observe_mapped_event(
        &AcpGatewayEvent::ConversationResolved {
            conversation_id: "conv-empty-turn".to_string(),
        },
        true,
    );
    diagnostics.observe_mapped_event(
        &AcpGatewayEvent::TurnComplete {
            outcome: "ok".to_string(),
        },
        true,
    );

    let context = test_context();
    let error = diagnostics
        .empty_turn_error_event(&context)
        .expect("planning chrome alone should surface empty turn error");
    match error {
        AcpGatewayEvent::Error {
            error_type: Some(error_type),
            ..
        } => assert_eq!(error_type, "empty_mapped_turn"),
        other => panic!("expected empty_mapped_turn error, got {other:?}"),
    }
}

#[tokio::test]
async fn assistant_text_delta_suppresses_empty_turn_error() {
    let mut diagnostics = AcpStreamDiagnostics::default();
    diagnostics.observe_runtime_event(&RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::AssistantTextDelta {
            text: "hello".to_string(),
        },
    ));

    let context = test_context();
    assert!(diagnostics.empty_turn_error_event(&context).is_none());
}

#[tokio::test]
async fn turn_complete_without_substantive_output_triggers_empty_turn_error() {
    let mut diagnostics = AcpStreamDiagnostics::default();
    diagnostics.observe_runtime_event(&RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::TurnCompleted { turn: None },
    ));
    diagnostics.observe_mapped_event(
        &AcpGatewayEvent::TurnComplete {
            outcome: "ok".to_string(),
        },
        true,
    );

    let context = test_context();
    assert!(diagnostics.empty_turn_error_event(&context).is_some());
}
