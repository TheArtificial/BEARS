use uuid::Uuid;

use crate::core::{
    acp_runtime::{
        ensure_acp_session_conversation_with_backend, AcpConversationSelectionSource,
        AcpConversationService,
    },
    acp_sessions::AcpSessionRow,
    runtime_contracts::{
        classify_runtime_error, runtime_error_is_conflict_pending_approval,
        runtime_error_is_no_active_runs_cancel, EnsureConversationRequest, RoleRuntimeBinding,
        RuntimeConversationBackend, RuntimeConversationRef, RuntimeErrorCategory,
        RuntimeHistoryPage,
    },
};
use crate::errors::CustomError;

struct MockConversationBackend {
    created_id: String,
    verify_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[allow(async_fn_in_trait)]
impl RuntimeConversationBackend for MockConversationBackend {
    async fn create_conversation(
        &self,
        _binding: &RoleRuntimeBinding,
    ) -> Result<RuntimeConversationRef, CustomError> {
        Ok(RuntimeConversationRef {
            id: self.created_id.clone(),
        })
    }

    async fn verify_conversation_belongs_to_binding(
        &self,
        _binding: &RoleRuntimeBinding,
        _conversation_id: &str,
    ) -> Result<(), CustomError> {
        self.verify_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn load_history(
        &self,
        _binding: &RoleRuntimeBinding,
        _conversation: &RuntimeConversationRef,
    ) -> Result<RuntimeHistoryPage, CustomError> {
        Ok(RuntimeHistoryPage {
            records: Vec::new(),
            raw_payload: None,
        })
    }
}

#[test]
fn runtime_error_classifier_maps_pending_approval_conflicts() {
    let err = CustomError::System(
        "Letta continue HTTP 409: waiting for approval".to_string(),
    );
    assert_eq!(
        classify_runtime_error(&err),
        RuntimeErrorCategory::ConflictPendingApproval
    );
    assert!(runtime_error_is_conflict_pending_approval(&err));
}

#[test]
fn runtime_error_classifier_maps_no_active_runs_cancel() {
    let err = CustomError::System("no active runs to cancel".to_string());
    assert!(runtime_error_is_no_active_runs_cancel(&err));
}

#[tokio::test]
async fn ensure_prompt_conversation_materializes_pending_new_selection() {
    let backend = MockConversationBackend {
        created_id: "conv-materialized-123".to_string(),
        verify_calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let binding = RoleRuntimeBinding {
        binding_id: "agent-materialize".to_string(),
        compatibility_backend: Some("runtime:letta".to_string()),
    };
    let pending_id = "new-acp-zed-abc123".to_string();
    let (resolution, result) = ensure_acp_session_conversation_with_backend(
        &backend,
        EnsureConversationRequest {
            bear_id: Uuid::nil(),
            role: "pair".to_string(),
            acp_session_id: "session-1".to_string(),
            requested_selection: Some(pending_id.clone()),
            binding: binding.clone(),
        },
        None,
        pending_id.clone(),
    )
    .await
    .expect("ensure conversation");

    assert!(result.created);
    assert_eq!(result.conversation.id, "conv-materialized-123");
    assert_eq!(resolution.upstream_target, "conv-materialized-123");
    assert_eq!(
        resolution.selection_source,
        AcpConversationSelectionSource::Explicit
    );
    assert_eq!(
        resolution.resolved_conversation.as_ref().map(|c| c.id.as_str()),
        Some("conv-materialized-123")
    );
}

#[tokio::test]
async fn ensure_prompt_conversation_reuses_resolved_session_conversation() {
    let backend = MockConversationBackend {
        created_id: "conv-unused".to_string(),
        verify_calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let binding = RoleRuntimeBinding {
        binding_id: "agent-resume".to_string(),
        compatibility_backend: Some("runtime:letta".to_string()),
    };
    let now = time::OffsetDateTime::now_utc();
    let session = AcpSessionRow {
        id: Uuid::nil(),
        user_id: 1,
        bear_id: Uuid::nil(),
        bear_slug: "bear".to_string(),
        acp_session_id: "session-1".to_string(),
        runtime_session_id: "runtime".to_string(),
        conversation_id: "conv-existing-456".to_string(),
        resolved_conversation_id: Some("conv-existing-456".to_string()),
        client: "zed".to_string(),
        cwd: None,
        adapter_environment: None,
        current_mode: "ask".to_string(),
        conversation_title: None,
        conversation_title_updated_at: None,
        conversation_title_synced_at: None,
        closed_at: None,
        archived_at: None,
        created_at: now,
        updated_at: now,
    };
    let (resolution, result) = ensure_acp_session_conversation_with_backend(
        &backend,
        EnsureConversationRequest {
            bear_id: Uuid::nil(),
            role: "pair".to_string(),
            acp_session_id: "session-1".to_string(),
            requested_selection: None,
            binding: binding.clone(),
        },
        Some(&session),
        "new-acp-zed-generated".to_string(),
    )
    .await
    .expect("ensure conversation");

    assert!(!result.created);
    assert_eq!(result.conversation.id, "conv-existing-456");
    assert_eq!(
        resolution.selection_source,
        AcpConversationSelectionSource::Resolved
    );
    assert!(!resolution.should_materialize_runtime_conversation);
}

#[sqlx::test]
async fn conversation_service_skips_backend_verify_for_canonical_rows(
    pool: sqlx::PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::core::{
        bears::{db::create_bear, db::BearParams},
        conversation_persistence::ensure_conversation_for_external_id,
    };

    let bear_id = create_bear(
        &pool,
        BearParams {
            slug: "epic-a2-conv-access",
            name: "Epic A2 Conv Access",
            description: "test",
            system_prompt: "test",
            default_model: None,
            tools_enabled: None,
            letta_agent_type: None,
            letta_tool_ids: sqlx::types::Json(vec![]),
            context_profile: None,
        },
    )
    .await?;
    let conversation = ensure_conversation_for_external_id(
        &pool,
        bear_id,
        None,
        "conv-canonical-access",
        None,
        None,
    )
    .await?;
    let _ = conversation;

    let config = crate::config::Config::test_stub();
    let letta = crate::core::letta::LettaClient::new(&config);
    let service = AcpConversationService::new(&pool, &config, &letta);
    service
        .verify_conversation_access(
            bear_id,
            &RoleRuntimeBinding {
                binding_id: "agent-test".to_string(),
                compatibility_backend: Some("runtime:letta".to_string()),
            },
            "conv-canonical-access",
        )
        .await?;

    Ok(())
}
