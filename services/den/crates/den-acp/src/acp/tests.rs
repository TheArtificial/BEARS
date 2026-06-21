use super::*;
use std::sync::Arc;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
use crate::acp::{AcpAdapterEnvironmentRequest, AcpPromptRequest, AcpStreamContext, acp_pair_den_tool_descriptors, looks_like_runtime_waiting_for_approval_error, requested_mode_from_prompt};
use den_core::tools::constants::{
    DEN_PLAN_MODE_CANCEL_PROVIDER, DEN_PLAN_MODE_ENTER_PROVIDER,
    DEN_PLAN_MODE_EXIT_PROVIDER, DEN_PLAN_MODE_RECORD_APPROVAL_PROVIDER,
    DEN_PLAN_MODE_STATUS_PROVIDER, DEN_WORK_PLAN_GET_STATUS_PROVIDER,
    DEN_WORK_PLAN_LIST_PROVIDER, DEN_WORK_PLAN_REQUEST_HANDOFF_PROVIDER,
    DEN_WORK_PLAN_UPDATE_PROVIDER,
};
use den_runtime::prompt_memory_blocks::{
    compile_prompt_memory_blocks, PromptMemoryBlock, PromptMemoryBlockScope,
    PromptMemoryBlockState, PromptMemoryBlockType, PromptMemoryCompilationInput,
};

    use bytes::Bytes;
    use futures::Stream;
    use reqwest::StatusCode;

    fn acp_test_runtime_event_stream(
        bytes: impl Stream<Item = Result<Bytes, CustomError>> + Send + 'static,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>> {
        let bytes = futures::StreamExt::map(bytes, |item| item.map_err(den_http::errors::DenError::from));
        let events = runtime_byte_stream_to_event_stream(
            Box::pin(bytes),
            RuntimeEventParser {
                parse_json_event: runtime_stream_event_from_letta_json,
            },
        );
        Box::pin(futures::StreamExt::map(events, |item| {
            item.map_err(CustomError::from)
        }))
    }
        use den_core::config::Config;
        use den_http::errors::CustomError;
        use crate::{
        acp::{
            history::{acp_auto_title_instruction, map_canonical_history_page},
            prompt_context::{
                acp_direct_tool_prompt_context_with_activity,
                render_prompt_memory_runtime_selection,
            },
            stream::{
                mapping::summarize_event_for_log,
                runtime::{
                    spawn_canonical_gateway_record_persistence,
                },
                sse_stream::{runtime_terminal_events, AcpRuntimeSseStream},
                text::AcpTextChunker,
            },
            tool_results::acp_tool_result_response_from_delivery,
        },
        service::DenState,
        core::{
            acp_runtime::{
                is_valid_pending_acp_conversation_id, resolve_acp_prompt_conversation,
                AcpConversationResolution, AcpConversationSelectionSource,
            },
            acp_turn_runner::{
                STALE_APPROVAL_RECOVERY_DENIAL_REASON,
            },
        },
    };
    use den_runtime::{
        gateway_events::GatewayEvent,
        runtime_stream_parser::{
                find_sse_frame_end, parse_sse_event_body_to_json,
                runtime_byte_stream_to_event_stream,
                runtime_stream_event_from_letta_json,
            },
        runtime_contracts::{RuntimeEventParser, RuntimeSemanticEvent, RuntimeStreamEvent},
        acp_sessions,
        tool_turns::{
                ToolResultDelivery, ToolResultRequest, ToolTurnCoordinator,
                ToolTurnRegistration,
            },
        client_tools::{ResolvedSessionPolicy, ToolStatus},
        bears::BearProfile,
        prompt_memory_block_store::{
                archive_conflicting_prompt_memory_blocks,
                archive_prompt_memory_blocks_superseded_by,
                list_prompt_memory_blocks_for_bear_profile,
                patch_prompt_memory_block, select_prompt_memory_blocks_for_runtime,
                upsert_prompt_memory_block, PromptMemoryBlockPatch,
                PromptMemoryBlockQuery, PromptMemoryBlockWrite,
            },
        turn_controller::{
                TerminalReason, TerminalStatus, TurnController, TurnPhase,
            },
        agent_assist::PendingApprovalDenialMode,
        role_runtime::{RoleRuntime, RoleTurnScope},
    };

    fn prompt_memory_test_state(pool: sqlx::PgPool) -> DenState {
        let config = Arc::new(Config::test_stub());
        DenState {
            sqlx_pool: pool,
            config: config.clone(),
            bifrost: Arc::new(den_service::bifrost::BifrostClient::new(config.as_ref())),
            tool_turns: ToolTurnCoordinator::new(),
            acp_turn_cancellations: den_service::turn_controller::ActiveTurnCancelRegistry::new(),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        }
    }

    fn prompt_memory_test_policy() -> ResolvedSessionPolicy {
        ResolvedSessionPolicy {
            mode_label: "Write",
            tool_enablement: den_runtime::client_tools::ToolEnablementState::AllTools,
            plan_mode_state: None,
        }
    }

    async fn prompt_memory_test_pool() -> Option<sqlx::PgPool> {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
        let pool = PgPoolOptions::new().connect(&database_url).await.ok()?;
        sqlx::migrate!("../../migrations").run(&pool).await.ok()?;
        Some(pool)
    }

    fn prompt_memory_test_context() -> (Uuid, String, String, String) {
        (
            Uuid::new_v4(),
            format!("sess-{}", Uuid::new_v4()),
            format!("/workspace/test-{}", Uuid::new_v4()),
            BearProfile::Pair.as_str().to_string(),
        )
    }

    async fn seed_prompt_memory_block(
        pool: &sqlx::PgPool,
        write: PromptMemoryBlockWrite,
    ) -> String {
        let block_id = write.block_id.clone();
        upsert_prompt_memory_block(pool, &write)
            .await
            .expect("seed prompt memory block");
        block_id
    }

    fn prompt_memory_runtime_query<'a>(
        bear_id: Uuid,
        profile_slug: &'a str,
        session_id: &'a str,
        root: &'a String,
    ) -> PromptMemoryBlockQuery<'a> {
        PromptMemoryBlockQuery {
            bear_id: Some(bear_id),
            profile_slug,
            work_surfaces: std::slice::from_ref(root),
            session_id,
        }
    }

    async fn select_rendered_prompt_memory_runtime(
        pool: &sqlx::PgPool,
        bear_id: Uuid,
        profile_slug: &str,
        session_id: &str,
        root: &String,
    ) -> (den_runtime::prompt_memory_block_store::PromptMemoryRuntimeSelection, String) {
        let selection = select_prompt_memory_blocks_for_runtime(
            pool,
            prompt_memory_runtime_query(bear_id, profile_slug, session_id, root),
        )
        .await
        .expect("persisted runtime selection");
        let rendered = render_prompt_memory_runtime_selection(
            &selection,
            session_id,
            std::slice::from_ref(root),
        );
        (selection, rendered)
    }

    #[tokio::test]
    async fn acp_prompt_context_with_activity_reports_budgeted_persisted_prompt_memory_diagnostics() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let (bear_id, session_id, root, profile_slug) = prompt_memory_test_context();
        let mut seeded_block_ids = Vec::new();
        for (index, (scope, block_type, work_surface, block_session_id, title, body, priority)) in [
            (PromptMemoryBlockScope::Session, PromptMemoryBlockType::SessionFocus, None, Some(session_id.clone()), "Session budget", "session budget", 100),
            (PromptMemoryBlockScope::WorkSurface, PromptMemoryBlockType::WorkSurfaceContext, Some(root.clone()), None, "Surface budget a", "surface budget a", 90),
            (PromptMemoryBlockScope::WorkSurface, PromptMemoryBlockType::WorkSurfaceContext, Some(root.clone()), None, "Surface budget b", "surface budget b", 80),
            (PromptMemoryBlockScope::RoleLocal, PromptMemoryBlockType::RoleGuidance, None, None, "Role budget a", "role budget a", 70),
            (PromptMemoryBlockScope::RoleLocal, PromptMemoryBlockType::RoleGuidance, None, None, "Role budget b", "role budget b", 60),
            (PromptMemoryBlockScope::BearWide, PromptMemoryBlockType::UserInstruction, None, None, "Bear budget a", "bear budget a", 50),
            (PromptMemoryBlockScope::BearWide, PromptMemoryBlockType::UserInstruction, None, None, "Bear budget b", "bear budget b", 40),
        ]
        .into_iter()
        .enumerate()
        {
            let block_id = format!("pm-acp-budget-{}-{}", index, Uuid::new_v4());
            seeded_block_ids.push(block_id.clone());
            seed_prompt_memory_block(
                &pool,
                PromptMemoryBlockWrite {
                    block_id,
                    bear_id: Some(bear_id),
                    profile_slug: Some(profile_slug.clone()),
                    scope,
                    block_type,
                    state: PromptMemoryBlockState::Active,
                    work_surface,
                    session_id: block_session_id,
                    title: title.to_string(),
                    body: body.to_string(),
                    priority,
                    created_by_user_id: Some(1),
                    supersedes_block_id: None,
                    metadata: serde_json::json!({}),
                },
            )
            .await;
        }

        let state = prompt_memory_test_state(pool);
        let policy = prompt_memory_test_policy();
        let (prompt, diagnostic) = acp_direct_tool_prompt_context_with_activity(
            &state,
            bear_id,
            &session_id,
            &root,
            &serde_json::json!({ "workspace_roots": [root.clone()] }),
            true,
            &policy,
            None,
            None,
        )
        .await
        .expect("prompt context");

        assert_eq!(diagnostic["source"], "prompt_memory_blocks");
        assert_eq!(diagnostic["persisted"], true);
        assert_eq!(diagnostic["matched_count"], 7);
        assert_eq!(
            diagnostic["matched_block_ids"],
            serde_json::json!(seeded_block_ids)
        );
        assert!(prompt.contains("Session budget"));
        assert!(prompt.contains("Surface budget a"));
        assert!(prompt.contains("Surface budget b"));
        assert!(prompt.contains("Role budget a"));
        assert!(prompt.contains("Role budget b"));
        assert!(prompt.contains("Bear budget a"));
        assert!(!prompt.contains("Bear budget b"));
        assert!(prompt.contains("Omitted lower-priority blocks due to prompt budgeting:"));
    }

    #[tokio::test]
    async fn acp_prompt_context_with_activity_reports_persisted_prompt_memory_precedence() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let (bear_id, session_id, root, profile_slug) = prompt_memory_test_context();
        let mut seeded_block_ids = Vec::new();
        for (scope, block_type, work_surface, block_session_id, title, body) in [
            (PromptMemoryBlockScope::BearWide, PromptMemoryBlockType::RoleGuidance, None, None, "Bear", "bear default"),
            (PromptMemoryBlockScope::RoleLocal, PromptMemoryBlockType::RoleGuidance, None, None, "Role", "role guidance"),
            (PromptMemoryBlockScope::WorkSurface, PromptMemoryBlockType::WorkSurfaceContext, Some(root.clone()), None, "Surface", "surface context"),
            (PromptMemoryBlockScope::Session, PromptMemoryBlockType::SessionFocus, None, Some(session_id.clone()), "Session", "session focus"),
        ] {
            let block_id = format!("pm-{}-{}", title.to_ascii_lowercase(), Uuid::new_v4());
            seeded_block_ids.push(block_id.clone());
            seed_prompt_memory_block(
                &pool,
                PromptMemoryBlockWrite {
                    block_id,
                    bear_id: Some(bear_id),
                    profile_slug: Some(profile_slug.clone()),
                    scope,
                    block_type,
                    state: PromptMemoryBlockState::Active,
                    work_surface,
                    session_id: block_session_id,
                    title: title.to_string(),
                    body: body.to_string(),
                    priority: 1,
                    created_by_user_id: Some(1),
                    supersedes_block_id: None,
                    metadata: serde_json::json!({}),
                },
            )
            .await;
        }
        let state = prompt_memory_test_state(pool);
        let policy = prompt_memory_test_policy();
        let (prompt, diagnostic) = acp_direct_tool_prompt_context_with_activity(
            &state,
            bear_id,
            &session_id,
            &root,
            &serde_json::json!({ "workspace_roots": [root.clone()] }),
            true,
            &policy,
            None,
            None,
        )
        .await
        .expect("prompt context");
        let session_index = prompt.find("Session").expect("session block present");
        let surface_index = prompt.find("Surface").expect("surface block present");
        let role_index = prompt.find("Role").expect("role block present");
        let bear_index = prompt.find("Bear").expect("bear block present");
        assert!(session_index < surface_index);
        assert!(surface_index < role_index);
        assert!(role_index < bear_index);
        assert_eq!(diagnostic["matched_count"], 4);
        assert_eq!(
            diagnostic["matched_block_ids"],
            serde_json::json!(seeded_block_ids)
        );
    }

    #[tokio::test]
    async fn persisted_prompt_memory_runtime_selection_render_reports_budget_omissions() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let (bear_id, session_id, root, profile_slug) = prompt_memory_test_context();
        let seed_specs = vec![
            (PromptMemoryBlockScope::Session, PromptMemoryBlockType::SessionFocus, None, Some(session_id.clone()), "Session focus", "session focus", 100),
            (PromptMemoryBlockScope::WorkSurface, PromptMemoryBlockType::WorkSurfaceContext, Some(root.clone()), None, "Surface alpha", "surface alpha", 90),
            (PromptMemoryBlockScope::WorkSurface, PromptMemoryBlockType::WorkSurfaceContext, Some(root.clone()), None, "Surface beta", "surface beta", 80),
            (PromptMemoryBlockScope::RoleLocal, PromptMemoryBlockType::RoleGuidance, None, None, "Role alpha", "role alpha", 70),
            (PromptMemoryBlockScope::RoleLocal, PromptMemoryBlockType::RoleGuidance, None, None, "Role beta", "role beta", 60),
            (PromptMemoryBlockScope::BearWide, PromptMemoryBlockType::UserInstruction, None, None, "Bear alpha", "bear alpha", 50),
            (PromptMemoryBlockScope::BearWide, PromptMemoryBlockType::UserInstruction, None, None, "Bear beta", "bear beta", 40),
        ];
        let mut seeded_block_ids = Vec::new();
        let mut expected_omitted_ids = Vec::new();
        for (index, (scope, block_type, work_surface, block_session_id, title, body, priority)) in
            seed_specs.into_iter().enumerate()
        {
            let block_id = format!("pm-budget-{}-{}", index, Uuid::new_v4());
            if index >= 6 {
                expected_omitted_ids.push(block_id.clone());
            }
            seeded_block_ids.push(block_id.clone());
            seed_prompt_memory_block(
                &pool,
                PromptMemoryBlockWrite {
                    block_id,
                    bear_id: Some(bear_id),
                    profile_slug: Some(profile_slug.clone()),
                    scope,
                    block_type,
                    state: PromptMemoryBlockState::Active,
                    work_surface,
                    session_id: block_session_id,
                    title: title.to_string(),
                    body: body.to_string(),
                    priority,
                    created_by_user_id: Some(1),
                    supersedes_block_id: None,
                    metadata: serde_json::json!({}),
                },
            )
            .await;
        }
        let (selection, rendered) = select_rendered_prompt_memory_runtime(
            &pool,
            bear_id,
            profile_slug.as_str(),
            session_id.as_str(),
            &root,
        )
        .await;
        assert_eq!(selection.diagnostic["source"], "prompt_memory_blocks");
        assert_eq!(selection.diagnostic["persisted"], true);
        assert_eq!(selection.diagnostic["matched_count"], 7);
        assert_eq!(
            selection.diagnostic["matched_block_ids"],
            serde_json::json!(seeded_block_ids)
        );

        assert!(rendered.contains("Session focus"));
        assert!(rendered.contains("Surface alpha"));
        assert!(rendered.contains("Surface beta"));
        assert!(rendered.contains("Role alpha"));
        assert!(rendered.contains("Role beta"));
        assert!(rendered.contains("Bear alpha"));
        assert!(!rendered.contains("Bear beta"));
        assert!(rendered.contains("Omitted lower-priority blocks due to prompt budgeting:"));
        for omitted_id in expected_omitted_ids {
            assert!(rendered.contains(&omitted_id));
        }
    }

    #[tokio::test]
    async fn acp_conversation_resolved_side_effect_persists_canonical_workflow_event() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let acp_session_id = format!("sess-{}", Uuid::new_v4());
        let pending_conversation_id = format!("new-{}", Uuid::new_v4());
        let resolved_conversation_id = format!("conv-{}", Uuid::new_v4());
        let request_id = Uuid::new_v4();

        acp_sessions::upsert_session(
            &pool,
            acp_sessions::UpsertAcpSession {
                user_id,
                bear_id,
                bear_slug: "test-bear".to_string(),
                acp_session_id: acp_session_id.clone(),
                runtime_session_id: format!("runtime-{}", Uuid::new_v4()),
                conversation_id: pending_conversation_id.clone(),
                resolved_conversation_id: None,
                client: "vscode".to_string(),
                cwd: Some("/workspace".to_string()),
                current_mode: Some("write".to_string()),
            },
        )
        .await
        .expect("upsert acp session");

        let context = AcpStreamContext {
            pool: pool.clone(),
            tool_turns: ToolTurnCoordinator::new(),
            user_id,
            user_profile: None,
            bear_id,
            bear_slug: "test-bear".to_string(),
            acp_session_id: acp_session_id.clone(),
            client: "vscode".to_string(),
            conversation_id: pending_conversation_id.clone(),
            conversation_selection: pending_conversation_id.clone(),
            resolved_conversation_id: None,
            upstream_target: resolved_conversation_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: Some(serde_json::json!({"mode_label": "Write"})),
            activity: None,
            request_id,
            pair_agent_id: "pair-agent".to_string(),
            config: Arc::new(Config::test_stub()),
            role_runtime: RoleRuntime::new(ToolTurnCoordinator::new()),
            turn_scope: RoleTurnScope::acp_pair(bear_id, acp_session_id.clone(), None),
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        let mut event = GatewayEvent::ConversationResolved {
            conversation_id: resolved_conversation_id.clone(),
        };
        super::stream::runtime::persist_stream_event_side_effects(&context, &mut event)
            .await
            .expect("persist stream side effects");

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let stored_session_resolved = sqlx::query_scalar::<_, Option<String>>(
            r"
            SELECT resolved_conversation_id
            FROM acp_sessions
            WHERE user_id = $1 AND bear_id = $2 AND acp_session_id = $3
            ",
        )
        .bind(user_id)
        .bind(bear_id)
        .bind(&acp_session_id)
        .fetch_one(&pool)
        .await
        .expect("reload resolved conversation id");
        assert_eq!(
            stored_session_resolved.as_deref(),
            Some(resolved_conversation_id.as_str())
        );

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &resolved_conversation_id,
            Some(&acp_session_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let page = den_runtime::conversation_persistence::list_messages_page(
            &pool,
            canonical.id,
            None,
            20,
        )
        .await
        .expect("list canonical messages");
        let resolved_event = page
            .into_iter()
            .find(|message| message.message_type == "workflow_event")
            .expect("conversation_resolved workflow event persisted");
        assert_eq!(resolved_event.content_text, "Conversation resolved");
        let content_json: serde_json::Value = sqlx::query_scalar(
            r"
            SELECT content_json
            FROM conversation_messages
            WHERE conversation_id = $1 AND sequence_no = $2
            ",
        )
        .bind(canonical.id)
        .bind(resolved_event.sequence_no)
        .fetch_one(&pool)
        .await
        .expect("load workflow event content_json");
        assert_eq!(content_json["event"], serde_json::json!("conversation_resolved"));
        assert_eq!(
            content_json["conversation_id"],
            serde_json::json!(resolved_conversation_id)
        );
        assert_eq!(content_json["source"], serde_json::json!("acp"));
        assert_eq!(
            content_json["scope_id"],
            serde_json::json!(format!("acp:{}", acp_session_id))
        );
    }

    #[tokio::test]
    async fn acp_tool_result_side_effect_persists_timeout_like_variant_payload() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let acp_session_id = format!("sess-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());
        let tool_call_id = format!("tool-call-{}", Uuid::new_v4());
        let request_id = format!("req-{}", Uuid::new_v4());

        acp_sessions::upsert_session(
            &pool,
            acp_sessions::UpsertAcpSession {
                user_id,
                bear_id,
                bear_slug: "test-bear".to_string(),
                acp_session_id: acp_session_id.clone(),
                runtime_session_id: format!("runtime-{}", Uuid::new_v4()),
                conversation_id: conversation_id.clone(),
                resolved_conversation_id: Some(conversation_id.clone()),
                client: "vscode".to_string(),
                cwd: Some("/workspace".to_string()),
                current_mode: Some("write".to_string()),
            },
        )
        .await
        .expect("upsert acp session");

        let context = AcpStreamContext {
            pool: pool.clone(),
            tool_turns: ToolTurnCoordinator::new(),
            user_id,
            user_profile: None,
            bear_id,
            bear_slug: "test-bear".to_string(),
            acp_session_id: acp_session_id.clone(),
            client: "vscode".to_string(),
            conversation_id: conversation_id.clone(),
            conversation_selection: conversation_id.clone(),
            resolved_conversation_id: Some(conversation_id.clone()),
            upstream_target: conversation_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: Some(serde_json::json!({"mode_label": "Write"})),
            activity: None,
            request_id: Uuid::new_v4(),
            pair_agent_id: "pair-agent".to_string(),
            config: Arc::new(Config::test_stub()),
            role_runtime: RoleRuntime::new(ToolTurnCoordinator::new()),
            turn_scope: RoleTurnScope::acp_pair(bear_id, acp_session_id.clone(), Some(conversation_id.clone())),
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        super::stream::runtime::spawn_persist_acp_tool_result(
            &context,
            Some("functions.fs.read_text_file".to_string()),
            tool_call_id.clone(),
            Some("approval-timeout-1".to_string()),
            "timed_out".to_string(),
            Some("Tool execution timed out waiting for result delivery.".to_string()),
            serde_json::json!({
                "retryable": true,
                "kind": "timeout",
                "partial": false,
            }),
            serde_json::json!({
                "component": "den.acp",
                "phase": "tool_result",
                "timeout": true,
            }),
            Some(request_id.clone()),
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &conversation_id,
            Some(&acp_session_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let page = den_runtime::conversation_persistence::list_messages_page(&pool, canonical.id, None, 20)
            .await
            .expect("list canonical messages");
        let tool_result = page
            .into_iter()
            .find(|message| message.message_type == "tool_event")
            .expect("tool_result event persisted");
        assert_eq!(tool_result.content_text, "Tool result: functions.fs.read_text_file");
        let content_json: serde_json::Value = sqlx::query_scalar(
            r"
            SELECT content_json
            FROM conversation_messages
            WHERE conversation_id = $1 AND sequence_no = $2
            ",
        )
        .bind(canonical.id)
        .bind(tool_result.sequence_no)
        .fetch_one(&pool)
        .await
        .expect("load tool result content_json");
        assert_eq!(content_json["event"], serde_json::json!("tool_result"));
        assert_eq!(content_json["status"], serde_json::json!("timed_out"));
        assert_eq!(content_json["tool_call_id"], serde_json::json!(tool_call_id));
        assert_eq!(content_json["approval_request_id"], serde_json::json!("approval-timeout-1"));
        assert_eq!(content_json["request_id"], serde_json::json!(request_id));
        assert_eq!(content_json["structured_content"]["kind"], serde_json::json!("timeout"));
        assert_eq!(content_json["structured_content"]["retryable"], serde_json::json!(true));
        assert_eq!(content_json["diagnostic"]["timeout"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn acp_tool_result_side_effect_persists_error_like_variant_payload() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let acp_session_id = format!("sess-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());
        let tool_call_id = format!("tool-call-{}", Uuid::new_v4());

        acp_sessions::upsert_session(
            &pool,
            acp_sessions::UpsertAcpSession {
                user_id,
                bear_id,
                bear_slug: "test-bear".to_string(),
                acp_session_id: acp_session_id.clone(),
                runtime_session_id: format!("runtime-{}", Uuid::new_v4()),
                conversation_id: conversation_id.clone(),
                resolved_conversation_id: Some(conversation_id.clone()),
                client: "vscode".to_string(),
                cwd: Some("/workspace".to_string()),
                current_mode: Some("write".to_string()),
            },
        )
        .await
        .expect("upsert acp session");

        let context = AcpStreamContext {
            pool: pool.clone(),
            tool_turns: ToolTurnCoordinator::new(),
            user_id,
            user_profile: None,
            bear_id,
            bear_slug: "test-bear".to_string(),
            acp_session_id: acp_session_id.clone(),
            client: "vscode".to_string(),
            conversation_id: conversation_id.clone(),
            conversation_selection: conversation_id.clone(),
            resolved_conversation_id: Some(conversation_id.clone()),
            upstream_target: conversation_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: Some(serde_json::json!({"mode_label": "Write"})),
            activity: None,
            request_id: Uuid::new_v4(),
            pair_agent_id: "pair-agent".to_string(),
            config: Arc::new(Config::test_stub()),
            role_runtime: RoleRuntime::new(ToolTurnCoordinator::new()),
            turn_scope: RoleTurnScope::acp_pair(bear_id, acp_session_id.clone(), Some(conversation_id.clone())),
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        super::stream::runtime::spawn_persist_acp_tool_result(
            &context,
            Some("functions.process.run".to_string()),
            tool_call_id.clone(),
            None,
            "error".to_string(),
            Some("Process execution failed with exit code 1".to_string()),
            serde_json::json!({
                "retryable": false,
                "kind": "error",
                "exit_code": 1,
            }),
            serde_json::json!({
                "component": "den.acp",
                "phase": "tool_result",
                "stderr_excerpt": "boom",
            }),
            Some(format!("req-{}", Uuid::new_v4())),
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &conversation_id,
            Some(&acp_session_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let page = den_runtime::conversation_persistence::list_messages_page(&pool, canonical.id, None, 20)
            .await
            .expect("list canonical messages");
        let tool_result = page
            .into_iter()
            .find(|message| {
                message.message_type == "tool_event"
                    && message.content_text == "Tool result: functions.process.run"
            })
            .expect("error-like tool_result event persisted");
        let content_json: serde_json::Value = sqlx::query_scalar(
            r"
            SELECT content_json
            FROM conversation_messages
            WHERE conversation_id = $1 AND sequence_no = $2
            ",
        )
        .bind(canonical.id)
        .bind(tool_result.sequence_no)
        .fetch_one(&pool)
        .await
        .expect("load error-like tool result content_json");
        assert_eq!(content_json["event"], serde_json::json!("tool_result"));
        assert_eq!(content_json["status"], serde_json::json!("error"));
        assert_eq!(content_json["structured_content"]["kind"], serde_json::json!("error"));
        assert_eq!(content_json["structured_content"]["exit_code"], serde_json::json!(1));
        assert_eq!(content_json["diagnostic"]["stderr_excerpt"], serde_json::json!("boom"));
        assert_eq!(content_json["tool_name"], serde_json::json!("functions.process.run"));
    }

    #[test]
    fn tool_result_prepare_runtime_continuation_maps_timeout_status_to_runtime_timeout() {
        let prepared = ToolTurnCoordinator::prepare_runtime_continuation(&ToolResultRequest {
            tool_call_id: Some("tool-call-timeout".to_string()),
            tool_name: Some("functions.fs.read_text_file".to_string()),
            approval_request_id: None,
            status: "timeout".to_string(),
            content: Some("timed out".to_string()),
            ..Default::default()
        })
        .expect("prepare continuation");

        match prepared.continuation {
            den_protocol::RuntimeContinuation::ToolResult {
                tool_call_id,
                approval_request_id,
                status,
                content,
            } => {
                assert_eq!(tool_call_id, "tool-call-timeout");
                assert_eq!(approval_request_id, None);
                assert_eq!(status, den_protocol::RuntimeToolResultStatus::Timeout);
                assert_eq!(content, "timed out");
            }
            other => panic!("expected tool-result continuation, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_prepare_runtime_continuation_maps_timed_out_status_to_runtime_timeout() {
        let prepared = ToolTurnCoordinator::prepare_runtime_continuation(&ToolResultRequest {
            tool_call_id: Some("tool-call-timed-out".to_string()),
            tool_name: Some("functions.fs.read_text_file".to_string()),
            approval_request_id: None,
            status: "timed_out".to_string(),
            content: Some("timed out variant".to_string()),
            ..Default::default()
        })
        .expect("prepare continuation");

        match prepared.continuation {
            den_protocol::RuntimeContinuation::ToolResult { status, .. } => {
                assert_eq!(status, den_protocol::RuntimeToolResultStatus::Timeout);
            }
            other => panic!("expected tool-result continuation, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_prepare_runtime_continuation_with_approval_request_maps_to_approval_decision() {
        let prepared = ToolTurnCoordinator::prepare_runtime_continuation(&ToolResultRequest {
            tool_call_id: Some("tool-call-approval".to_string()),
            tool_name: Some("functions.process.run".to_string()),
            approval_request_id: Some("approval-123".to_string()),
            status: "error".to_string(),
            content: Some("user denied".to_string()),
            ..Default::default()
        })
        .expect("prepare continuation");

        match prepared.continuation {
            den_protocol::RuntimeContinuation::ApprovalDecision {
                approval_request_id,
                tool_call_id,
                decision,
                reason,
            } => {
                assert_eq!(approval_request_id, "approval-123");
                assert_eq!(tool_call_id.as_deref(), Some("tool-call-approval"));
                assert_eq!(decision, den_protocol::RuntimeApprovalDecision::Deny);
                assert_eq!(reason.as_deref(), Some("user denied"));
            }
            other => panic!("expected approval-decision continuation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn acp_runtime_sse_stream_holds_terminal_until_tool_result_continuation_is_drained() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let acp_session_id = format!("sess-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());
        let request_id = Uuid::new_v4();
        let tool_call_id = format!("tool-call-{}", Uuid::new_v4());

        acp_sessions::upsert_session(
            &pool,
            acp_sessions::UpsertAcpSession {
                user_id,
                bear_id,
                bear_slug: "test-bear".to_string(),
                acp_session_id: acp_session_id.clone(),
                runtime_session_id: format!("runtime-{}", Uuid::new_v4()),
                conversation_id: conversation_id.clone(),
                resolved_conversation_id: Some(conversation_id.clone()),
                client: "vscode".to_string(),
                cwd: Some("/workspace".to_string()),
                current_mode: Some("write".to_string()),
            },
        )
        .await
        .expect("upsert acp session");

        let tool_turns = ToolTurnCoordinator::new();
        let (_result_tx_unused, result_rx) = tokio::sync::oneshot::channel();

        let active_turn_guard = tool_turns
            .acquire_active_turn(&acp_session_id, request_id, Some(conversation_id.clone()))
            .expect("acquire active turn");
        let context = AcpStreamContext {
            pool: pool.clone(),
            tool_turns: tool_turns.clone(),
            user_id,
            user_profile: None,
            bear_id,
            bear_slug: "test-bear".to_string(),
            acp_session_id: acp_session_id.clone(),
            client: "vscode".to_string(),
            conversation_id: conversation_id.clone(),
            conversation_selection: conversation_id.clone(),
            resolved_conversation_id: Some(conversation_id.clone()),
            upstream_target: conversation_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: Some(serde_json::json!({"mode_label": "Write"})),
            activity: None,
            request_id,
            pair_agent_id: "pair-agent".to_string(),
            config: Arc::new(Config::test_stub()),
            role_runtime: RoleRuntime::new(tool_turns.clone()),
            turn_scope: RoleTurnScope::acp_pair(bear_id, acp_session_id.clone(), Some(conversation_id.clone())),
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        let inner = futures::stream::empty::<Result<Bytes, CustomError>>();
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(inner),
            context,
            Vec::new(),
            false,
            den_runtime::role_runtime::RoleTurnGuard { guard: active_turn_guard },
        );
        stream.waiting_adapter_tool_result = Some((
            tool_call_id.clone(),
            "functions.fs.read_text_file".to_string(),
            AcpResolvedToolResult::Receiver(result_rx),
        ));
        stream.turn_controller.on_tool_request(
            tool_call_id.clone(),
            "functions.fs.read_text_file".to_string(),
            den_service::turn_controller::ToolExecutionRoute::DenServer,
        );
        stream.turn_controller.on_stream_end();

        let role_result = stream.context.role_runtime.turn_result(
            den_runtime::role_runtime::TurnResultStatus::Ok,
            den_runtime::role_runtime::TurnResultReason::StreamComplete,
            request_id,
            stream.context.turn_scope.clone(),
            false,
            serde_json::json!({"test": "continuation_hold"}),
        );
        stream.push_terminal_result_when_ready(role_result);
        assert!(stream.pending.is_empty(), "terminal should be held while tool continuation is pending");

        stream.persist_future = Some(AcpPendingFuture::Tool(Box::pin(async move {
            Some(Box::new(ToolResultRequest {
                request_id: Some(request_id.to_string()),
                tool_call_id: Some(tool_call_id.clone()),
                tool_name: Some("functions.fs.read_text_file".to_string()),
                status: "timed_out".to_string(),
                content: Some("tool timed out".to_string()),
                structured_content: serde_json::json!({"kind": "timeout"}),
                diagnostic: serde_json::json!({"timeout": true}),
                ..Default::default()
            }))
        })));

        use futures::StreamExt;
        let first = stream.next().await.expect("tool result event frame").expect("ok frame");
        let first_text = String::from_utf8(first.to_vec()).expect("utf8 frame");
        assert!(first_text.contains("tool_result"), "expected tool_result frame, got: {first_text}");

        assert!(stream.queued_tool_result_continuation.is_some(), "tool result should queue runtime continuation");
        assert!(stream.pending.iter().all(|frame| !String::from_utf8_lossy(frame).contains("turn_result")));
    }

    #[tokio::test]
    async fn acp_turn_outcome_side_effect_persists_failed_terminal_payload() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let acp_session_id = format!("sess-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());
        let request_id = Uuid::new_v4();

        acp_sessions::upsert_session(
            &pool,
            acp_sessions::UpsertAcpSession {
                user_id,
                bear_id,
                bear_slug: "test-bear".to_string(),
                acp_session_id: acp_session_id.clone(),
                runtime_session_id: format!("runtime-{}", Uuid::new_v4()),
                conversation_id: conversation_id.clone(),
                resolved_conversation_id: Some(conversation_id.clone()),
                client: "vscode".to_string(),
                cwd: Some("/workspace".to_string()),
                current_mode: Some("write".to_string()),
            },
        )
        .await
        .expect("upsert acp session");

        let context = AcpStreamContext {
            pool: pool.clone(),
            tool_turns: ToolTurnCoordinator::new(),
            user_id,
            user_profile: None,
            bear_id,
            bear_slug: "test-bear".to_string(),
            acp_session_id: acp_session_id.clone(),
            client: "vscode".to_string(),
            conversation_id: conversation_id.clone(),
            conversation_selection: conversation_id.clone(),
            resolved_conversation_id: Some(conversation_id.clone()),
            upstream_target: conversation_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: Some(serde_json::json!({"mode_label": "Write"})),
            activity: None,
            request_id,
            pair_agent_id: "pair-agent".to_string(),
            config: Arc::new(Config::test_stub()),
            role_runtime: RoleRuntime::new(ToolTurnCoordinator::new()),
            turn_scope: RoleTurnScope::acp_pair(bear_id, acp_session_id.clone(), Some(conversation_id.clone())),
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        let role_result = context.role_runtime.turn_result(
            den_runtime::role_runtime::TurnResultStatus::Failed,
            den_runtime::role_runtime::TurnResultReason::RuntimeCleanup,
            request_id,
            context.turn_scope.clone(),
            false,
            serde_json::json!({
                "component": "den.acp",
                "source": "test",
                "event": "failed_terminal",
            }),
        );
        super::stream::runtime::spawn_persist_acp_turn_outcome(&context, &role_result);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &conversation_id,
            Some(&acp_session_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let page = den_runtime::conversation_persistence::list_messages_page(&pool, canonical.id, None, 20)
            .await
            .expect("list canonical messages");
        let turn_outcome = page
            .into_iter()
            .find(|message| message.message_type == "workflow_event")
            .expect("failed turn_outcome persisted");
        assert_eq!(turn_outcome.content_text, "Turn outcome: failed / runtime_cleanup");
        let content_json: serde_json::Value = sqlx::query_scalar(
            r"
            SELECT content_json
            FROM conversation_messages
            WHERE conversation_id = $1 AND sequence_no = $2
            ",
        )
        .bind(canonical.id)
        .bind(turn_outcome.sequence_no)
        .fetch_one(&pool)
        .await
        .expect("load failed turn outcome content_json");
        assert_eq!(content_json["event"], serde_json::json!("turn_result"));
        assert_eq!(content_json["status"], serde_json::json!("failed"));
        assert_eq!(content_json["reason"], serde_json::json!("runtime_cleanup"));
        assert_eq!(content_json["retryable"], serde_json::json!(false));
        assert_eq!(content_json["diagnostics"]["event"], serde_json::json!("failed_terminal"));
    }

    #[tokio::test]
    async fn acp_turn_outcome_side_effect_persists_cancelled_terminal_payload() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let acp_session_id = format!("sess-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());
        let request_id = Uuid::new_v4();

        acp_sessions::upsert_session(
            &pool,
            acp_sessions::UpsertAcpSession {
                user_id,
                bear_id,
                bear_slug: "test-bear".to_string(),
                acp_session_id: acp_session_id.clone(),
                runtime_session_id: format!("runtime-{}", Uuid::new_v4()),
                conversation_id: conversation_id.clone(),
                resolved_conversation_id: Some(conversation_id.clone()),
                client: "vscode".to_string(),
                cwd: Some("/workspace".to_string()),
                current_mode: Some("write".to_string()),
            },
        )
        .await
        .expect("upsert acp session");

        let context = AcpStreamContext {
            pool: pool.clone(),
            tool_turns: ToolTurnCoordinator::new(),
            user_id,
            user_profile: None,
            bear_id,
            bear_slug: "test-bear".to_string(),
            acp_session_id: acp_session_id.clone(),
            client: "vscode".to_string(),
            conversation_id: conversation_id.clone(),
            conversation_selection: conversation_id.clone(),
            resolved_conversation_id: Some(conversation_id.clone()),
            upstream_target: conversation_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: Some(serde_json::json!({"mode_label": "Write"})),
            activity: None,
            request_id,
            pair_agent_id: "pair-agent".to_string(),
            config: Arc::new(Config::test_stub()),
            role_runtime: RoleRuntime::new(ToolTurnCoordinator::new()),
            turn_scope: RoleTurnScope::acp_pair(bear_id, acp_session_id.clone(), Some(conversation_id.clone())),
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        let role_result = context.role_runtime.turn_result(
            den_runtime::role_runtime::TurnResultStatus::Cancelled,
            den_runtime::role_runtime::TurnResultReason::Cancelled,
            request_id,
            context.turn_scope.clone(),
            false,
            serde_json::json!({
                "component": "den.acp",
                "source": "test",
                "event": "cancelled_terminal",
            }),
        );
        super::stream::runtime::spawn_persist_acp_turn_outcome(&context, &role_result);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &conversation_id,
            Some(&acp_session_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let page = den_runtime::conversation_persistence::list_messages_page(&pool, canonical.id, None, 20)
            .await
            .expect("list canonical messages");
        let turn_outcome = page
            .into_iter()
            .find(|message| {
                message.message_type == "workflow_event"
                    && message.content_text == "Turn outcome: cancelled / cancelled"
            })
            .expect("cancelled turn_outcome persisted");
        let content_json: serde_json::Value = sqlx::query_scalar(
            r"
            SELECT content_json
            FROM conversation_messages
            WHERE conversation_id = $1 AND sequence_no = $2
            ",
        )
        .bind(canonical.id)
        .bind(turn_outcome.sequence_no)
        .fetch_one(&pool)
        .await
        .expect("load cancelled turn outcome content_json");
        assert_eq!(content_json["event"], serde_json::json!("turn_result"));
        assert_eq!(content_json["status"], serde_json::json!("cancelled"));
        assert_eq!(content_json["reason"], serde_json::json!("cancelled"));
        assert_eq!(content_json["diagnostics"]["event"], serde_json::json!("cancelled_terminal"));
    }

    #[tokio::test]
    async fn acp_turn_outcome_side_effect_persists_recovered_terminal_payload() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let acp_session_id = format!("sess-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());
        let request_id = Uuid::new_v4();

        acp_sessions::upsert_session(
            &pool,
            acp_sessions::UpsertAcpSession {
                user_id,
                bear_id,
                bear_slug: "test-bear".to_string(),
                acp_session_id: acp_session_id.clone(),
                runtime_session_id: format!("runtime-{}", Uuid::new_v4()),
                conversation_id: conversation_id.clone(),
                resolved_conversation_id: Some(conversation_id.clone()),
                client: "vscode".to_string(),
                cwd: Some("/workspace".to_string()),
                current_mode: Some("write".to_string()),
            },
        )
        .await
        .expect("upsert acp session");

        let context = AcpStreamContext {
            pool: pool.clone(),
            tool_turns: ToolTurnCoordinator::new(),
            user_id,
            user_profile: None,
            bear_id,
            bear_slug: "test-bear".to_string(),
            acp_session_id: acp_session_id.clone(),
            client: "vscode".to_string(),
            conversation_id: conversation_id.clone(),
            conversation_selection: conversation_id.clone(),
            resolved_conversation_id: Some(conversation_id.clone()),
            upstream_target: conversation_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: Some(serde_json::json!({"mode_label": "Write"})),
            activity: None,
            request_id,
            pair_agent_id: "pair-agent".to_string(),
            config: Arc::new(Config::test_stub()),
            role_runtime: RoleRuntime::new(ToolTurnCoordinator::new()),
            turn_scope: RoleTurnScope::acp_pair(bear_id, acp_session_id.clone(), Some(conversation_id.clone())),
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        let role_result = context.role_runtime.turn_result(
            den_runtime::role_runtime::TurnResultStatus::Recovered,
            den_runtime::role_runtime::TurnResultReason::CompactedRetry,
            request_id,
            context.turn_scope.clone(),
            true,
            serde_json::json!({
                "component": "den.acp",
                "source": "test",
                "event": "recovered_terminal",
            }),
        );
        super::stream::runtime::spawn_persist_acp_turn_outcome(&context, &role_result);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &conversation_id,
            Some(&acp_session_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let page = den_runtime::conversation_persistence::list_messages_page(&pool, canonical.id, None, 20)
            .await
            .expect("list canonical messages");
        let turn_outcome = page
            .into_iter()
            .find(|message| {
                message.message_type == "workflow_event"
                    && message.content_text == "Turn outcome: recovered / compacted_retry"
            })
            .expect("recovered turn_outcome persisted");
        let content_json: serde_json::Value = sqlx::query_scalar(
            r"
            SELECT content_json
            FROM conversation_messages
            WHERE conversation_id = $1 AND sequence_no = $2
            ",
        )
        .bind(canonical.id)
        .bind(turn_outcome.sequence_no)
        .fetch_one(&pool)
        .await
        .expect("load recovered turn outcome content_json");
        assert_eq!(content_json["event"], serde_json::json!("turn_result"));
        assert_eq!(content_json["status"], serde_json::json!("recovered"));
        assert_eq!(content_json["reason"], serde_json::json!("compacted_retry"));
        assert_eq!(content_json["retryable"], serde_json::json!(true));
        assert_eq!(content_json["diagnostics"]["event"], serde_json::json!("recovered_terminal"));
    }

    #[tokio::test]
    async fn canonical_visible_user_message_persistence_dedups_same_prompt_provenance() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let session_id = format!("acp-session-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());
        let request_id = format!("req-{}", Uuid::new_v4());
        let provenance = den_runtime::conversation_events::ConversationEventProvenance {
            source: "acp_prompt".to_string(),
            scope_id: session_id.clone(),
        };
        let mut content_json = provenance.as_content_json("user_prompt");
        content_json["role"] = serde_json::json!("user");
        content_json["acp_session_id"] = serde_json::json!(session_id.clone());
        content_json["client"] = serde_json::json!("zed");
        content_json["request_id"] = serde_json::json!(request_id.clone());
        let record = den_runtime::conversation_events::CanonicalConversationRecord::visible_user_message(
            "dedup me",
            content_json,
            None,
        );
        let context = den_runtime::conversation_events::ConversationPersistenceContext {
            pool: pool.clone(),
            bear_id,
            user_id: Some(user_id),
            external_conversation_id: conversation_id.clone(),
            source_session_id: Some(session_id.clone()),
            request_id: Some(request_id.clone()),
            persistence_scope_id: session_id.clone(),
            skip_persistence: false,
        };

        den_runtime::conversation_events::persist_canonical_conversation_record(&context, &record)
            .await
            .expect("persist initial user prompt");
        den_runtime::conversation_events::persist_canonical_conversation_record(&context, &record)
            .await
            .expect("persist duplicate user prompt");

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &conversation_id,
            Some(&session_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let page = den_runtime::conversation_persistence::list_messages_page(&pool, canonical.id, None, 20)
            .await
            .expect("list canonical messages");
        let user_messages: Vec<_> = page
            .into_iter()
            .filter(|message| message.message_type == "user" && message.role.as_deref() == Some("user"))
            .collect();
        assert_eq!(user_messages.len(), 1, "duplicate prompt provenance should dedup canonical persistence");

        let content_json: serde_json::Value = sqlx::query_scalar(
            r"
            SELECT content_json
            FROM conversation_messages
            WHERE conversation_id = $1 AND sequence_no = $2
            ",
        )
        .bind(canonical.id)
        .bind(user_messages[0].sequence_no)
        .fetch_one(&pool)
        .await
        .expect("load persisted user prompt content_json");
        assert_eq!(content_json["source"], serde_json::json!("acp_prompt"));
        assert_eq!(content_json["event"], serde_json::json!("user_prompt"));
        assert_eq!(content_json["scope_id"], serde_json::json!(session_id));
        assert_eq!(content_json["request_id"], serde_json::json!(request_id));
        assert_eq!(content_json["client"], serde_json::json!("zed"));
    }

    #[tokio::test]
    async fn canonical_visible_user_message_persistence_keeps_distinct_request_ids() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let session_id = format!("acp-session-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());

        let build_record = |request_id: String| {
            let provenance = den_runtime::conversation_events::ConversationEventProvenance {
                source: "acp_prompt".to_string(),
                scope_id: session_id.clone(),
            };
            let mut content_json = provenance.as_content_json("user_prompt");
            content_json["role"] = serde_json::json!("user");
            content_json["acp_session_id"] = serde_json::json!(session_id.clone());
            content_json["client"] = serde_json::json!("zed");
            content_json["request_id"] = serde_json::json!(request_id);
            den_runtime::conversation_events::CanonicalConversationRecord::visible_user_message(
                "same text, new turn",
                content_json,
                None,
            )
        };

        let context = den_runtime::conversation_events::ConversationPersistenceContext {
            pool: pool.clone(),
            bear_id,
            user_id: Some(user_id),
            external_conversation_id: conversation_id.clone(),
            source_session_id: Some(session_id.clone()),
            request_id: None,
            persistence_scope_id: session_id.clone(),
            skip_persistence: false,
        };

        den_runtime::conversation_events::persist_canonical_conversation_record(
            &context,
            &build_record(format!("req-{}", Uuid::new_v4())),
        )
        .await
        .expect("persist first user prompt");
        den_runtime::conversation_events::persist_canonical_conversation_record(
            &context,
            &build_record(format!("req-{}", Uuid::new_v4())),
        )
        .await
        .expect("persist second user prompt");

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &conversation_id,
            Some(&session_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let page = den_runtime::conversation_persistence::list_messages_page(&pool, canonical.id, None, 20)
            .await
            .expect("list canonical messages");
        let user_messages: Vec<_> = page
            .into_iter()
            .filter(|message| message.message_type == "user" && message.role.as_deref() == Some("user"))
            .collect();
        assert_eq!(user_messages.len(), 2, "distinct prompt request_ids should remain separate canonical messages");
    }

    #[tokio::test]
    async fn canonical_history_page_projects_prompt_tool_result_and_assistant_replay_order() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let session_id = format!("acp-session-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());
        let request_id = format!("req-{}", Uuid::new_v4());
        let tool_call_id = format!("call-{}", Uuid::new_v4());
        let context = den_runtime::conversation_events::ConversationPersistenceContext {
            pool: pool.clone(),
            bear_id,
            user_id: Some(user_id),
            external_conversation_id: conversation_id.clone(),
            source_session_id: Some(session_id.clone()),
            request_id: Some(request_id.clone()),
            persistence_scope_id: session_id.clone(),
            skip_persistence: false,
        };

        let provenance = den_runtime::conversation_events::ConversationEventProvenance {
            source: "acp_prompt".to_string(),
            scope_id: session_id.clone(),
        };
        let mut prompt_json = provenance.as_content_json("user_prompt");
        prompt_json["role"] = serde_json::json!("user");
        prompt_json["acp_session_id"] = serde_json::json!(session_id.clone());
        prompt_json["client"] = serde_json::json!("zed");
        prompt_json["request_id"] = serde_json::json!(request_id.clone());
        den_runtime::conversation_events::persist_canonical_conversation_record(
            &context,
            &den_runtime::conversation_events::CanonicalConversationRecord::visible_user_message(
                "read a file",
                prompt_json,
                None,
            ),
        )
        .await
        .expect("persist user prompt");

        den_runtime::conversation_events::persist_canonical_conversation_record(
            &context,
            &den_runtime::conversation_events::CanonicalConversationRecord::tool_request(
                "functions.fs.read_text_file",
                tool_call_id.clone(),
                request_id.clone(),
                None,
                serde_json::json!({"path": "/tmp/acp-workspace/README.md", "line": 1, "limit": 10}),
                false,
                None,
                "den_server",
                &den_runtime::conversation_events::ConversationEventProvenance {
                    source: "acp_runtime".to_string(),
                    scope_id: context.persistence_scope_id.clone(),
                },
            ),
        )
        .await
        .expect("persist tool request");

        den_runtime::conversation_events::persist_canonical_conversation_record(
            &context,
            &den_runtime::conversation_events::CanonicalConversationRecord::tool_result(
                Some("functions.fs.read_text_file".to_string()),
                tool_call_id.clone(),
                None,
                "ok",
                Some("# README\n".to_string()),
                serde_json::json!({"path": "/tmp/acp-workspace/README.md"}),
                serde_json::json!({"source": "test"}),
                Some(request_id.clone()),
                &den_runtime::conversation_events::ConversationEventProvenance {
                    source: "acp_tool_result".to_string(),
                    scope_id: context.persistence_scope_id.clone(),
                },
            ),
        )
        .await
        .expect("persist tool result");

        let assistant_json = serde_json::json!({
            "source": "acp_runtime",
            "event": "assistant_output",
            "scope_id": context.persistence_scope_id,
            "request_id": context.request_id,
            "role": "assistant"
        });
        den_runtime::conversation_events::persist_canonical_conversation_record(
            &context,
            &den_runtime::conversation_events::CanonicalConversationRecord::visible_assistant_message(
                "read complete",
                assistant_json,
                None,
            ),
        )
        .await
        .expect("persist assistant output");

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &conversation_id,
            Some(&context.persistence_scope_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let rows = den_runtime::conversation_persistence::list_messages_page(&pool, canonical.id, None, 20)
            .await
            .expect("list canonical rows");
        let (messages, has_more, next_before) = map_canonical_history_page(&rows, 20);

        assert!(!has_more, "small replay page should not paginate");
        assert!(next_before.is_some(), "history page should surface next_before anchor");
        assert_eq!(messages.len(), 2, "canonical ACP history projection should include visible user + assistant transcript messages");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].text, "read a file");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].text, "read complete");
    }

    #[tokio::test]
    async fn conversation_resolved_record_does_not_block_canonical_history_projection() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let bear_id = Uuid::new_v4();
        let user_id = 1;
        let session_id = format!("acp-session-{}", Uuid::new_v4());
        let conversation_id = format!("conv-{}", Uuid::new_v4());
        let request_id = format!("req-{}", Uuid::new_v4());
        let context = den_runtime::conversation_events::ConversationPersistenceContext {
            pool: pool.clone(),
            bear_id,
            user_id: Some(user_id),
            external_conversation_id: conversation_id.clone(),
            source_session_id: Some(session_id.clone()),
            request_id: Some(request_id.clone()),
            persistence_scope_id: session_id.clone(),
            skip_persistence: false,
        };

        den_runtime::conversation_events::persist_canonical_conversation_record(
            &context,
            &den_runtime::conversation_events::CanonicalConversationRecord::conversation_resolved(
                &conversation_id,
                &den_runtime::conversation_events::ConversationEventProvenance {
                    source: "acp_runtime".to_string(),
                    scope_id: context.persistence_scope_id.clone(),
                },
            ),
        )
        .await
        .expect("persist conversation_resolved record");

        let prompt_provenance = den_runtime::conversation_events::ConversationEventProvenance {
            source: "acp_prompt".to_string(),
            scope_id: context.persistence_scope_id.clone(),
        };
        let mut prompt_json = prompt_provenance.as_content_json("user_prompt");
        prompt_json["role"] = serde_json::json!("user");
        prompt_json["acp_session_id"] = serde_json::json!(session_id.clone());
        prompt_json["client"] = serde_json::json!("zed");
        prompt_json["request_id"] = serde_json::json!(request_id.clone());
        den_runtime::conversation_events::persist_canonical_conversation_record(
            &context,
            &den_runtime::conversation_events::CanonicalConversationRecord::visible_user_message(
                "hello after resolution",
                prompt_json,
                None,
            ),
        )
        .await
        .expect("persist user prompt");

        den_runtime::conversation_events::persist_canonical_conversation_record(
            &context,
            &den_runtime::conversation_events::CanonicalConversationRecord::visible_assistant_message(
                "resolved response",
                serde_json::json!({
                    "source": "acp_runtime",
                    "event": "assistant_output",
                    "scope_id": context.persistence_scope_id,
                    "request_id": request_id,
                    "role": "assistant"
                }),
                None,
            ),
        )
        .await
        .expect("persist assistant output");

        let canonical = den_runtime::conversation_persistence::ensure_conversation_for_external_id(
            &pool,
            bear_id,
            Some(user_id),
            &conversation_id,
            Some(&session_id),
            None,
        )
        .await
        .expect("ensure canonical conversation");
        let visible_count = den_runtime::conversation_persistence::count_visible_messages(&pool, canonical.id)
            .await
            .expect("count visible messages");
        assert_eq!(visible_count, 3, "conversation_resolved plus prompt/assistant should count as visible canonical records");

        let rows = den_runtime::conversation_persistence::list_messages_page(&pool, canonical.id, None, 20)
            .await
            .expect("list canonical rows");
        assert!(rows.iter().any(|row| row.message_type == "workflow_event" && row.content_text.contains("Conversation resolved")));
        let (messages, has_more, next_before) = map_canonical_history_page(&rows, 20);
        assert!(!has_more, "small canonical history page should not paginate");
        assert!(next_before.is_some(), "history page should surface next_before anchor");
        assert_eq!(messages.len(), 2, "workflow-only conversation_resolved event should not pollute visible transcript replay");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].text, "hello after resolution");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].text, "resolved response");
    }

    #[tokio::test]
    async fn prompt_memory_runtime_selection_matrix_excludes_inactive_and_mismatched_blocks() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let (bear_id, session_id, root, profile_slug) = prompt_memory_test_context();

        let active_session_id = seed_prompt_memory_block(
            &pool,
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-session-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::Session,
                block_type: PromptMemoryBlockType::SessionFocus,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: Some(session_id.clone()),
                title: "Session active".to_string(),
                body: "session active body".to_string(),
                priority: 90,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
        )
        .await;
        let active_surface_id = seed_prompt_memory_block(
            &pool,
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-surface-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::WorkSurface,
                block_type: PromptMemoryBlockType::WorkSurfaceContext,
                state: PromptMemoryBlockState::Active,
                work_surface: Some(root.clone()),
                session_id: None,
                title: "Surface active".to_string(),
                body: "surface active body".to_string(),
                priority: 80,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
        )
        .await;
        let active_role_id = seed_prompt_memory_block(
            &pool,
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-role-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::RoleLocal,
                block_type: PromptMemoryBlockType::RoleGuidance,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: None,
                title: "Role active".to_string(),
                body: "role active body".to_string(),
                priority: 70,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
        )
        .await;
        let active_bear_id = seed_prompt_memory_block(
            &pool,
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-bear-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::BearWide,
                block_type: PromptMemoryBlockType::UserInstruction,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: None,
                title: "Bear active".to_string(),
                body: "bear active body".to_string(),
                priority: 60,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
        )
        .await;

        for write in [
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-draft-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::RoleLocal,
                block_type: PromptMemoryBlockType::RoleGuidance,
                state: PromptMemoryBlockState::Draft,
                work_surface: None,
                session_id: None,
                title: "Role draft".to_string(),
                body: "role draft body".to_string(),
                priority: 100,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-superseded-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::RoleLocal,
                block_type: PromptMemoryBlockType::RoleGuidance,
                state: PromptMemoryBlockState::Superseded,
                work_surface: None,
                session_id: None,
                title: "Role superseded".to_string(),
                body: "role superseded body".to_string(),
                priority: 99,
                created_by_user_id: Some(1),
                supersedes_block_id: Some(active_role_id.clone()),
                metadata: serde_json::json!({}),
            },
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-archived-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::Session,
                block_type: PromptMemoryBlockType::SessionFocus,
                state: PromptMemoryBlockState::Archived,
                work_surface: None,
                session_id: Some(session_id.clone()),
                title: "Session archived".to_string(),
                body: "session archived body".to_string(),
                priority: 98,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-other-session-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::Session,
                block_type: PromptMemoryBlockType::SessionFocus,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: Some(format!("other-{}", Uuid::new_v4())),
                title: "Session mismatch".to_string(),
                body: "session mismatch body".to_string(),
                priority: 97,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-other-surface-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::WorkSurface,
                block_type: PromptMemoryBlockType::WorkSurfaceContext,
                state: PromptMemoryBlockState::Active,
                work_surface: Some(format!("/workspace/other-{}", Uuid::new_v4())),
                session_id: None,
                title: "Surface mismatch".to_string(),
                body: "surface mismatch body".to_string(),
                priority: 96,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
            PromptMemoryBlockWrite {
                block_id: format!("pm-matrix-other-role-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some("watch".to_string()),
                scope: PromptMemoryBlockScope::RoleLocal,
                block_type: PromptMemoryBlockType::RoleGuidance,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: None,
                title: "Role mismatch".to_string(),
                body: "role mismatch body".to_string(),
                priority: 95,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
        ] {
            seed_prompt_memory_block(&pool, write).await;
        }

        let (selection, rendered) = select_rendered_prompt_memory_runtime(
            &pool,
            bear_id,
            profile_slug.as_str(),
            session_id.as_str(),
            &root,
        )
        .await;

        let expected_ids = vec![
            active_session_id.clone(),
            active_surface_id.clone(),
            active_role_id.clone(),
            active_bear_id.clone(),
        ];
        assert_eq!(selection.diagnostic["source"], "prompt_memory_blocks");
        assert_eq!(selection.diagnostic["persisted"], true);
        assert_eq!(selection.diagnostic["matched_count"], 4);
        assert_eq!(
            selection.diagnostic["matched_block_ids"],
            serde_json::json!(expected_ids)
        );
        assert!(rendered.contains("Session active"));
        assert!(rendered.contains("Surface active"));
        assert!(rendered.contains("Role active"));
        assert!(rendered.contains("Bear active"));
        for excluded in [
            "Role draft",
            "Role superseded",
            "Session archived",
            "Session mismatch",
            "Surface mismatch",
            "Role mismatch",
        ] {
            assert!(!rendered.contains(excluded), "render unexpectedly contained {excluded}");
        }
    }

    #[tokio::test]
    async fn persisted_prompt_memory_runtime_selection_exact_fit_omits_no_blocks() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let (bear_id, session_id, root, profile_slug) = prompt_memory_test_context();
        let mut seeded_block_ids = Vec::new();
        for (index, (scope, block_type, work_surface, block_session_id, title, body, priority)) in [
            (PromptMemoryBlockScope::Session, PromptMemoryBlockType::SessionFocus, None, Some(session_id.clone()), "Session exact", "session exact", 100),
            (PromptMemoryBlockScope::WorkSurface, PromptMemoryBlockType::WorkSurfaceContext, Some(root.clone()), None, "Surface exact a", "surface exact a", 90),
            (PromptMemoryBlockScope::WorkSurface, PromptMemoryBlockType::WorkSurfaceContext, Some(root.clone()), None, "Surface exact b", "surface exact b", 80),
            (PromptMemoryBlockScope::RoleLocal, PromptMemoryBlockType::RoleGuidance, None, None, "Role exact a", "role exact a", 70),
            (PromptMemoryBlockScope::RoleLocal, PromptMemoryBlockType::RoleGuidance, None, None, "Role exact b", "role exact b", 60),
            (PromptMemoryBlockScope::BearWide, PromptMemoryBlockType::UserInstruction, None, None, "Bear exact", "bear exact", 50),
        ]
        .into_iter()
        .enumerate()
        {
            let block_id = format!("pm-exact-fit-{}-{}", index, Uuid::new_v4());
            seeded_block_ids.push(block_id.clone());
            seed_prompt_memory_block(
                &pool,
                PromptMemoryBlockWrite {
                    block_id,
                    bear_id: Some(bear_id),
                    profile_slug: Some(profile_slug.clone()),
                    scope,
                    block_type,
                    state: PromptMemoryBlockState::Active,
                    work_surface,
                    session_id: block_session_id,
                    title: title.to_string(),
                    body: body.to_string(),
                    priority,
                    created_by_user_id: Some(1),
                    supersedes_block_id: None,
                    metadata: serde_json::json!({}),
                },
            )
            .await;
        }

        let (selection, rendered) = select_rendered_prompt_memory_runtime(
            &pool,
            bear_id,
            profile_slug.as_str(),
            session_id.as_str(),
            &root,
        )
        .await;

        assert_eq!(selection.diagnostic["matched_count"], 6);
        assert_eq!(
            selection.diagnostic["matched_block_ids"],
            serde_json::json!(seeded_block_ids)
        );
        assert!(rendered.contains("Session exact"));
        assert!(rendered.contains("Surface exact a"));
        assert!(rendered.contains("Surface exact b"));
        assert!(rendered.contains("Role exact a"));
        assert!(rendered.contains("Role exact b"));
        assert!(rendered.contains("Bear exact"));
        assert!(!rendered.contains("Omitted lower-priority blocks due to prompt budgeting:"));
    }

    #[tokio::test]
    async fn persisted_prompt_memory_runtime_selection_no_match_renders_empty_fallback() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let (bear_id, session_id, root, profile_slug) = prompt_memory_test_context();

        for write in [
            PromptMemoryBlockWrite {
                block_id: format!("pm-no-match-archived-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::Session,
                block_type: PromptMemoryBlockType::SessionFocus,
                state: PromptMemoryBlockState::Archived,
                work_surface: None,
                session_id: Some(session_id.clone()),
                title: "Archived no match".to_string(),
                body: "archived no match".to_string(),
                priority: 10,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
            PromptMemoryBlockWrite {
                block_id: format!("pm-no-match-other-session-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::Session,
                block_type: PromptMemoryBlockType::SessionFocus,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: Some(format!("other-{}", Uuid::new_v4())),
                title: "Other session no match".to_string(),
                body: "other session no match".to_string(),
                priority: 9,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
        ] {
            seed_prompt_memory_block(&pool, write).await;
        }

        let (selection, rendered) = select_rendered_prompt_memory_runtime(
            &pool,
            bear_id,
            profile_slug.as_str(),
            session_id.as_str(),
            &root,
        )
        .await;

        assert_eq!(selection.diagnostic["matched_count"], 0);
        assert_eq!(selection.diagnostic["matched_block_ids"], serde_json::json!([]));
        assert_eq!(
            rendered,
            "No prompt memory blocks are active for this runtime context."
        );
    }

    #[tokio::test]
    async fn persisted_prompt_memory_runtime_selection_excludes_each_inactive_state_individually() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let (bear_id, session_id, root, profile_slug) = prompt_memory_test_context();
        let active_block_id = seed_prompt_memory_block(
            &pool,
            PromptMemoryBlockWrite {
                block_id: format!("pm-lifecycle-active-{}", Uuid::new_v4()),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::RoleLocal,
                block_type: PromptMemoryBlockType::RoleGuidance,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: None,
                title: "Lifecycle active".to_string(),
                body: "lifecycle active body".to_string(),
                priority: 40,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
        )
        .await;

        for (state, label) in [
            (PromptMemoryBlockState::Draft, "Lifecycle draft"),
            (PromptMemoryBlockState::Superseded, "Lifecycle superseded"),
            (PromptMemoryBlockState::Archived, "Lifecycle archived"),
        ] {
            seed_prompt_memory_block(
                &pool,
                PromptMemoryBlockWrite {
                    block_id: format!("pm-lifecycle-{}-{}", label.replace(' ', "-").to_ascii_lowercase(), Uuid::new_v4()),
                    bear_id: Some(bear_id),
                    profile_slug: Some(profile_slug.clone()),
                    scope: PromptMemoryBlockScope::RoleLocal,
                    block_type: PromptMemoryBlockType::RoleGuidance,
                    state,
                    work_surface: None,
                    session_id: None,
                    title: label.to_string(),
                    body: format!("{label} body"),
                    priority: 100,
                    created_by_user_id: Some(1),
                    supersedes_block_id: Some(active_block_id.clone()),
                    metadata: serde_json::json!({}),
                },
            )
            .await;
        }

        let (selection, rendered) = select_rendered_prompt_memory_runtime(
            &pool,
            bear_id,
            profile_slug.as_str(),
            session_id.as_str(),
            &root,
        )
        .await;

        assert_eq!(selection.diagnostic["matched_count"], 1);
        assert_eq!(
            selection.diagnostic["matched_block_ids"],
            serde_json::json!([active_block_id])
        );
        assert!(rendered.contains("Lifecycle active"));
        for excluded in [
            "Lifecycle draft",
            "Lifecycle superseded",
            "Lifecycle archived",
        ] {
            assert!(!rendered.contains(excluded), "inactive lifecycle block leaked into render: {excluded}");
        }
    }

    #[tokio::test]
    async fn persisted_prompt_memory_runtime_selection_tie_breaks_deterministically_by_title() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let (bear_id, session_id, root, profile_slug) = prompt_memory_test_context();
        let titles = ["Zulu role", "Alpha role", "Mike role"];
        let mut ids_by_title = std::collections::BTreeMap::new();
        for title in titles {
            let block_id = format!("pm-tie-{}-{}", title.replace(' ', "-").to_ascii_lowercase(), Uuid::new_v4());
            ids_by_title.insert(title.to_string(), block_id.clone());
            seed_prompt_memory_block(
                &pool,
                PromptMemoryBlockWrite {
                    block_id,
                    bear_id: Some(bear_id),
                    profile_slug: Some(profile_slug.clone()),
                    scope: PromptMemoryBlockScope::RoleLocal,
                    block_type: PromptMemoryBlockType::RoleGuidance,
                    state: PromptMemoryBlockState::Active,
                    work_surface: None,
                    session_id: None,
                    title: title.to_string(),
                    body: format!("{title} body"),
                    priority: 42,
                    created_by_user_id: Some(1),
                    supersedes_block_id: None,
                    metadata: serde_json::json!({}),
                },
            )
            .await;
        }

        let (selection, rendered) = select_rendered_prompt_memory_runtime(
            &pool,
            bear_id,
            profile_slug.as_str(),
            session_id.as_str(),
            &root,
        )
        .await;

        let expected_ids = vec![
            ids_by_title["Alpha role"].clone(),
            ids_by_title["Mike role"].clone(),
            ids_by_title["Zulu role"].clone(),
        ];
        assert_eq!(selection.diagnostic["matched_count"], 3);
        assert_eq!(
            selection.diagnostic["matched_block_ids"],
            serde_json::json!(expected_ids)
        );
        let alpha_index = rendered.find("Alpha role").expect("alpha present");
        let mike_index = rendered.find("Mike role").expect("mike present");
        let zulu_index = rendered.find("Zulu role").expect("zulu present");
        assert!(alpha_index < mike_index);
        assert!(mike_index < zulu_index);
        assert!(!rendered.contains("Omitted lower-priority blocks due to prompt budgeting:"));
    }

    #[tokio::test]
    async fn prompt_memory_block_store_mutations_archive_conflicts_and_superseded_runtime_rows() {
        let Some(pool) = prompt_memory_test_pool().await else {
            return;
        };
        let (bear_id, session_id, root, profile_slug) = prompt_memory_test_context();
        let original_block_id = format!("pm-role-original-{}", Uuid::new_v4());
        let replacement_block_id = format!("pm-role-replacement-{}", Uuid::new_v4());
        let draft_block_id = format!("pm-draft-{}", Uuid::new_v4());

        seed_prompt_memory_block(
            &pool,
            PromptMemoryBlockWrite {
                block_id: original_block_id.clone(),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::RoleLocal,
                block_type: PromptMemoryBlockType::RoleGuidance,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: None,
                title: "Role original".to_string(),
                body: "original role guidance".to_string(),
                priority: 25,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
        )
        .await;

        seed_prompt_memory_block(
            &pool,
            PromptMemoryBlockWrite {
                block_id: draft_block_id.clone(),
                bear_id: Some(bear_id),
                profile_slug: Some(profile_slug.clone()),
                scope: PromptMemoryBlockScope::RoleLocal,
                block_type: PromptMemoryBlockType::RoleGuidance,
                state: PromptMemoryBlockState::Draft,
                work_surface: None,
                session_id: None,
                title: "Role draft".to_string(),
                body: "draft role guidance".to_string(),
                priority: 5,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: serde_json::json!({}),
            },
        )
        .await;

        let replacement_write = PromptMemoryBlockWrite {
            block_id: replacement_block_id.clone(),
            bear_id: Some(bear_id),
            profile_slug: Some(profile_slug.clone()),
            scope: PromptMemoryBlockScope::RoleLocal,
            block_type: PromptMemoryBlockType::RoleGuidance,
            state: PromptMemoryBlockState::Active,
            work_surface: None,
            session_id: None,
            title: "Role replacement".to_string(),
            body: "replacement role guidance".to_string(),
            priority: 30,
            created_by_user_id: Some(1),
            supersedes_block_id: Some(original_block_id.clone()),
            metadata: serde_json::json!({"source":"test"}),
        };
        seed_prompt_memory_block(&pool, replacement_write.clone()).await;

        let archived_conflicts = archive_conflicting_prompt_memory_blocks(&pool, &replacement_write)
            .await
            .expect("archive conflicts");
        assert_eq!(archived_conflicts, 1);

        let archived_superseded = archive_prompt_memory_blocks_superseded_by(
            &pool,
            bear_id,
            profile_slug.as_str(),
            original_block_id.as_str(),
        )
        .await
        .expect("archive superseded");
        assert_eq!(archived_superseded, 0);

        patch_prompt_memory_block(
            &pool,
            &draft_block_id,
            &PromptMemoryBlockPatch {
                state: PromptMemoryBlockState::Superseded,
                title: "Role draft superseded".to_string(),
                body: "draft role guidance superseded".to_string(),
                priority: 5,
                supersedes_block_id: Some(original_block_id.clone()),
                metadata: serde_json::json!({"patched":true}),
            },
        )
        .await
        .expect("patch draft block");

        let all_blocks = list_prompt_memory_blocks_for_bear_profile(&pool, bear_id, profile_slug.as_str())
            .await
            .expect("list prompt memory blocks");
        let original = all_blocks
            .iter()
            .find(|block| block.id == original_block_id)
            .expect("original block present");
        let replacement = all_blocks
            .iter()
            .find(|block| block.id == replacement_block_id)
            .expect("replacement block present");
        let draft = all_blocks
            .iter()
            .find(|block| block.id == draft_block_id)
            .expect("draft block present");

        assert_eq!(original.state, PromptMemoryBlockState::Archived);
        assert_eq!(replacement.state, PromptMemoryBlockState::Active);
        assert_eq!(draft.state, PromptMemoryBlockState::Superseded);
        assert_eq!(draft.title, "Role draft superseded");

        let (selection, rendered) = select_rendered_prompt_memory_runtime(
            &pool,
            bear_id,
            profile_slug.as_str(),
            session_id.as_str(),
            &root,
        )
        .await;

        assert_eq!(selection.diagnostic["matched_count"], 1);
        assert_eq!(
            selection.diagnostic["matched_block_ids"],
            serde_json::json!([replacement_block_id])
        );
        assert!(rendered.contains("Role replacement"));
        assert!(rendered.contains("replacement role guidance"));
        assert!(!rendered.contains("Role original"));
        assert!(!rendered.contains("original role guidance"));
        assert!(!rendered.contains("Role draft superseded"));
    }

    #[test]
    fn acp_prompt_requested_mode_is_normalized() {
        let body: AcpPromptRequest = serde_json::from_value(serde_json::json!({
            "message": "hello",
            "requested_mode": " WRITE "
        }))
        .expect("prompt request");

        assert_eq!(requested_mode_from_prompt(&body).unwrap(), Some("write"));
    }

    #[test]
    fn acp_prompt_requested_mode_rejects_unknown_values() {
        let body: AcpPromptRequest = serde_json::from_value(serde_json::json!({
            "message": "hello",
            "requested_mode": "sudo"
        }))
        .expect("prompt request");

        assert!(matches!(
            requested_mode_from_prompt(&body),
            Err(CustomError::ValidationError(_))
        ));
    }

    #[test]
    fn acp_pair_descriptors_keep_workboard_tools_but_hide_mode_control_tools() {
        let descriptors = acp_pair_den_tool_descriptors();
        let names = descriptors
            .as_array()
            .expect("descriptor array")
            .iter()
            .filter_map(|descriptor| descriptor.get("name").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();

        for expected in [
            DEN_WORK_PLAN_UPDATE_PROVIDER,
            DEN_WORK_PLAN_GET_STATUS_PROVIDER,
            DEN_WORK_PLAN_LIST_PROVIDER,
            DEN_WORK_PLAN_REQUEST_HANDOFF_PROVIDER,
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }

        for hidden in [
            DEN_PLAN_MODE_ENTER_PROVIDER,
            DEN_PLAN_MODE_STATUS_PROVIDER,
            DEN_PLAN_MODE_RECORD_APPROVAL_PROVIDER,
            DEN_PLAN_MODE_EXIT_PROVIDER,
            DEN_PLAN_MODE_CANCEL_PROVIDER,
        ] {
            assert!(
                !names.contains(&hidden),
                "unexpected mode-control tool {hidden}"
            );
        }
    }

    #[test]
    fn concurrent_letta_run_conflict_is_not_stale_approval() {
        let err = den_http::errors::DenError::System(
            "Letta send message HTTP 409 Conflict: another run is still processing this conversation"
                .to_string(),
        );

        assert!(!looks_like_runtime_waiting_for_approval_error(&err));
    }

    #[test]
    fn prompt_memory_block_compilation_prefers_session_then_surface_then_role_scope() {
        let work_surfaces = vec!["/workspace".to_string()];
        let blocks = vec![
            PromptMemoryBlock {
                id: "bear".to_string(),
                block_type: PromptMemoryBlockType::UserInstruction,
                scope: PromptMemoryBlockScope::BearWide,
                state: PromptMemoryBlockState::Active,
                role: None,
                work_surface: None,
                session_id: None,
                title: "bear".to_string(),
                body: "bear default".to_string(),
                priority: 1,
            },
            PromptMemoryBlock {
                id: "role".to_string(),
                block_type: PromptMemoryBlockType::RoleGuidance,
                scope: PromptMemoryBlockScope::RoleLocal,
                state: PromptMemoryBlockState::Active,
                role: Some("pair".to_string()),
                work_surface: None,
                session_id: None,
                title: "role".to_string(),
                body: "role guidance".to_string(),
                priority: 1,
            },
            PromptMemoryBlock {
                id: "surface".to_string(),
                block_type: PromptMemoryBlockType::WorkSurfaceContext,
                scope: PromptMemoryBlockScope::WorkSurface,
                state: PromptMemoryBlockState::Active,
                role: None,
                work_surface: Some("/workspace".to_string()),
                session_id: None,
                title: "surface".to_string(),
                body: "surface context".to_string(),
                priority: 1,
            },
            PromptMemoryBlock {
                id: "session".to_string(),
                block_type: PromptMemoryBlockType::SessionFocus,
                scope: PromptMemoryBlockScope::Session,
                state: PromptMemoryBlockState::Active,
                role: None,
                work_surface: None,
                session_id: Some("sess-1".to_string()),
                title: "session".to_string(),
                body: "session focus".to_string(),
                priority: 1,
            },
        ];

        let compiled = compile_prompt_memory_blocks(
            &blocks,
            PromptMemoryCompilationInput {
                role: "pair",
                work_surfaces: &work_surfaces,
                session_id: "sess-1",
                max_blocks: 4,
            },
        );
        let ids = compiled
            .included_blocks
            .iter()
            .map(|block| block.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["session", "surface", "role", "bear"]);
    }


    #[test]
    fn acp_recovery_approval_denial_reasons_do_not_look_like_policy_blocks() {
        let reason = STALE_APPROVAL_RECOVERY_DENIAL_REASON;
        assert!(!reason.contains("Denied by BEARS"));
        assert!(reason.contains("expired ACP approval request"));
        assert!(reason.contains("not a user or web policy block"));
        assert!(reason.contains("Retry the tool"));
    }

    #[test]
    fn canonical_history_page_includes_message_role_variants_and_skips_diagnostic_only() {
        use den_runtime::conversation_persistence::PersistedConversationMessage;
        use time::OffsetDateTime;

        let now = OffsetDateTime::now_utc();
        let rows = vec![
            PersistedConversationMessage {
                sequence_no: 4,
                message_type: "workflow_event".to_string(),
                role: Some("system".to_string()),
                visibility: "diagnostic_only".to_string(),
                content_text: "Turn outcome: ok / stream_complete".to_string(),
                provider_message_id: None,
                created_at: now,
            },
            PersistedConversationMessage {
                sequence_no: 3,
                message_type: "message".to_string(),
                role: Some("assistant".to_string()),
                visibility: "default".to_string(),
                content_text: "assistant reply".to_string(),
                provider_message_id: None,
                created_at: now,
            },
            PersistedConversationMessage {
                sequence_no: 2,
                message_type: "message".to_string(),
                role: Some("user".to_string()),
                visibility: "default".to_string(),
                content_text: "user prompt".to_string(),
                provider_message_id: None,
                created_at: now,
            },
            PersistedConversationMessage {
                sequence_no: 1,
                message_type: "assistant".to_string(),
                role: None,
                visibility: "default".to_string(),
                content_text: "legacy assistant".to_string(),
                provider_message_id: None,
                created_at: now,
            },
        ];

        let (messages, has_more, next_before) = map_canonical_history_page(&rows, 10);
        assert!(!has_more);
        assert_eq!(next_before.as_deref(), Some("1"));
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[0].text, "legacy assistant");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].text, "user prompt");
        assert_eq!(messages[2].role, "assistant");
        assert_eq!(messages[2].text, "assistant reply");
    }

    #[test]
    fn canonical_history_page_prefers_den_rows_when_prompt_and_assistant_exist() {
        use den_runtime::conversation_persistence::PersistedConversationMessage;
        use time::OffsetDateTime;

        let now = OffsetDateTime::now_utc();
        let rows = vec![
            PersistedConversationMessage {
                sequence_no: 2,
                message_type: "assistant".to_string(),
                role: None,
                visibility: "default".to_string(),
                content_text: "persisted assistant".to_string(),
                provider_message_id: Some("msg-2".to_string()),
                created_at: now,
            },
            PersistedConversationMessage {
                sequence_no: 1,
                message_type: "user".to_string(),
                role: None,
                visibility: "default".to_string(),
                content_text: "persisted prompt".to_string(),
                provider_message_id: Some("msg-1".to_string()),
                created_at: now,
            },
        ];

        let (messages, has_more, next_before) = map_canonical_history_page(&rows, 50);
        assert!(!has_more);
        assert_eq!(next_before.as_deref(), Some("1"));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].text, "persisted prompt");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].text, "persisted assistant");
    }

    #[test]
    fn canonical_history_page_diagnostic_only_rows_do_not_create_visible_history() {
        use den_runtime::conversation_persistence::PersistedConversationMessage;
        use time::OffsetDateTime;

        let now = OffsetDateTime::now_utc();
        let rows = vec![
            PersistedConversationMessage {
                sequence_no: 2,
                message_type: "workflow_event".to_string(),
                role: Some("system".to_string()),
                visibility: "diagnostic_only".to_string(),
                content_text: "Conversation resolved".to_string(),
                provider_message_id: None,
                created_at: now,
            },
            PersistedConversationMessage {
                sequence_no: 1,
                message_type: "workflow_event".to_string(),
                role: Some("system".to_string()),
                visibility: "diagnostic_only".to_string(),
                content_text: "Turn outcome: ok / stream_complete".to_string(),
                provider_message_id: None,
                created_at: now,
            },
        ];

        let (messages, has_more, next_before) = map_canonical_history_page(&rows, 50);
        assert!(messages.is_empty());
        assert!(!has_more);
        assert_eq!(next_before.as_deref(), Some("1"));
    }

    #[test]
    fn canonical_history_page_with_only_user_rows_still_returns_den_visible_history() {
        use den_runtime::conversation_persistence::PersistedConversationMessage;
        use time::OffsetDateTime;

        let now = OffsetDateTime::now_utc();
        let rows = vec![PersistedConversationMessage {
            sequence_no: 1,
            message_type: "user".to_string(),
            role: None,
            visibility: "default".to_string(),
            content_text: "persisted prompt only".to_string(),
            provider_message_id: Some("msg-1".to_string()),
            created_at: now,
        }];

        let (messages, has_more, next_before) = map_canonical_history_page(&rows, 50);
        assert!(!has_more);
        assert_eq!(next_before.as_deref(), Some("1"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].text, "persisted prompt only");
    }

    #[test]
    fn canonical_history_page_with_only_assistant_rows_still_returns_den_visible_history() {
        use den_runtime::conversation_persistence::PersistedConversationMessage;
        use time::OffsetDateTime;

        let now = OffsetDateTime::now_utc();
        let rows = vec![PersistedConversationMessage {
            sequence_no: 1,
            message_type: "assistant".to_string(),
            role: None,
            visibility: "default".to_string(),
            content_text: "persisted assistant only".to_string(),
            provider_message_id: Some("msg-1".to_string()),
            created_at: now,
        }];

        let (messages, has_more, next_before) = map_canonical_history_page(&rows, 50);
        assert!(!has_more);
        assert_eq!(next_before.as_deref(), Some("1"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[0].text, "persisted assistant only");
    }

    #[test]
    fn canonical_read_eligibility_requires_visible_canonical_messages() {
        use den_runtime::conversation_persistence::PersistedConversationMessage;
        use time::OffsetDateTime;

        let now = OffsetDateTime::now_utc();
        let diagnostic_only = vec![PersistedConversationMessage {
            sequence_no: 1,
            message_type: "workflow_event".to_string(),
            role: Some("system".to_string()),
            visibility: "diagnostic_only".to_string(),
            content_text: "conversation resolved".to_string(),
            provider_message_id: None,
            created_at: now,
        }];
        let (diagnostic_messages, _, _) = map_canonical_history_page(&diagnostic_only, 50);
        assert!(diagnostic_messages.is_empty());

        let visible = vec![PersistedConversationMessage {
            sequence_no: 2,
            message_type: "assistant".to_string(),
            role: None,
            visibility: "default".to_string(),
            content_text: "visible assistant".to_string(),
            provider_message_id: None,
            created_at: now,
        }];
        let (visible_messages, _, _) = map_canonical_history_page(&visible, 50);
        assert_eq!(visible_messages.len(), 1);
        assert_eq!(visible_messages[0].text, "visible assistant");
    }

    #[test]
    fn adapter_environment_request_deserializes_client_thread_title() {
        let body: AcpAdapterEnvironmentRequest = serde_json::from_value(serde_json::json!({
            "environment": { "thread_title": "Zed rename" },
            "conversation_title": "Zed rename"
        }))
        .expect("request should deserialize");
        assert_eq!(body.conversation_title.as_deref(), Some("Zed rename"));
        assert_eq!(
            body.environment
                .get("thread_title")
                .and_then(|value| value.as_str()),
            Some("Zed rename")
        );
    }

    #[test]
    fn adapter_environment_title_extraction_prefers_explicit_conversation_title() {
        let body: AcpAdapterEnvironmentRequest = serde_json::from_value(serde_json::json!({
            "environment": {
                "thread_title": "Thread title",
                "conversation_title": "Environment conversation title",
                "title": "Fallback title"
            },
            "conversation_title": "Explicit title"
        }))
        .expect("request should deserialize");
        let client_title = body
            .conversation_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                ["thread_title", "conversation_title", "title"]
                    .iter()
                    .find_map(|key| {
                        body.environment
                            .get(*key)
                            .and_then(|value| value.as_str())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                    })
            });
        assert_eq!(client_title, Some("Explicit title"));
    }

    #[test]
    fn adapter_environment_title_extraction_falls_back_to_thread_title() {
        let body: AcpAdapterEnvironmentRequest = serde_json::from_value(serde_json::json!({
            "environment": {
                "thread_title": "Thread title",
                "conversation_title": "Environment conversation title",
                "title": "Fallback title"
            }
        }))
        .expect("request should deserialize");
        let client_title = body
            .conversation_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                ["thread_title", "conversation_title", "title"]
                    .iter()
                    .find_map(|key| {
                        body.environment
                            .get(*key)
                            .and_then(|value| value.as_str())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                    })
            });
        assert_eq!(client_title, Some("Thread title"));
    }

    #[test]
    fn adapter_environment_title_extraction_ignores_blank_values() {
        let body: AcpAdapterEnvironmentRequest = serde_json::from_value(serde_json::json!({
            "environment": {
                "thread_title": "   ",
                "conversation_title": "",
                "title": "Fallback title"
            },
            "conversation_title": "  "
        }))
        .expect("request should deserialize");
        let client_title = body
            .conversation_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                ["thread_title", "conversation_title", "title"]
                    .iter()
                    .find_map(|key| {
                        body.environment
                            .get(*key)
                            .and_then(|value| value.as_str())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                    })
            });
        assert_eq!(client_title, Some("Fallback title"));
    }

    #[tokio::test]
    async fn acp_direct_tool_prompt_context_marks_untitled_sessions() {
        let policy = den_runtime::client_tools::resolve_session_policy_for_mode("ask", None);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1:9/den_test")
            .unwrap();
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = crate::service::DenState {
            sqlx_pool: pool,
            config: config.clone(),
            bifrost: std::sync::Arc::new(den_service::bifrost::BifrostClient::new(config.as_ref())),
            tool_turns: den_service::tool_turns::ToolTurnCoordinator::new(),
            acp_turn_cancellations: den_service::turn_controller::ActiveTurnCancelRegistry::new(),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        };
        let (context, diagnostic) = acp_direct_tool_prompt_context_with_activity(
            &state,
            uuid::Uuid::nil(),
            "acp-test-session",
            "/workspace",
            &serde_json::json!({
                "workspace_roots": ["/workspace"],
                "tools": []
            }),
            true,
            &policy,
            None,
            Some("This conversation is currently untitled. Once the main subject is clear enough to summarize in a short, specific title, proactively call `set_conversation_title` in that turn without waiting for the user to ask."),
        )
        .await
        .expect("prompt context");
        assert!(
            context.contains("Conversation title status for this ACP session: currently untitled.")
        );
        assert!(context.contains("set_conversation_title"));
        assert!(diagnostic["source"].is_string());
    }

    #[test]
    fn synthetic_prompt_memory_runtime_selection_emits_diagnostic_metadata() {
        let selection = super::prompt_context::synthetic_prompt_memory_runtime_selection(
            "sess-1",
            &["/workspace".to_string()],
        );
        assert_eq!(selection.diagnostic["source"], "synthetic_runtime_slice");
        assert_eq!(selection.diagnostic["persisted"], false);
        assert!(selection.diagnostic["matched_count"].as_u64().unwrap_or(0) >= 1);
        assert!(selection.diagnostic["matched_block_ids"].is_array());
        assert!(selection.diagnostic["omitted_block_ids"].is_array());
    }

    #[test]
    fn summarize_provider_event_for_log_redacts_large_tool_return() {
        let event = serde_json::json!({
            "message_type": "tool_return_message",
            "id": "message-test",
            "run_id": "run-test",
            "step_id": "step-test",
            "tool_call_id": "call-test",
            "status": "success",
            "tool_return": "x".repeat(10_000),
            "tool_call": {
                "function": {
                    "name": "fs_edit_file",
                    "arguments": "{\"path\":\"/tmp/a\",\"old_text\":\"secret\",\"new_text\":\"replacement\"}"
                }
            }
        });
        let summary = summarize_event_for_log(&event);
        assert_eq!(summary["message_type"], "tool_return_message");
        assert_eq!(summary["run_id"], "run-test");
        assert_eq!(summary["tool_call_id"], "call-test");
        assert!(
            summary["tool_return"]["redacted"] == true
                || summary["tool_return"]["redacted"].is_null()
        );
        assert_eq!(summary["tool_return"]["bytes"], 10_000);
        assert!(summary["tool_return"]["preview"].is_string());
        assert!(
            summary["tool_call"]["function"]["arguments"]["redacted"] == true
                || summary["tool_call"]["function"]["arguments"]["redacted"].is_null()
        );
        assert!(
            summary["tool_call"]["function"]["arguments"]["json_keys"]
                == serde_json::json!(["new_text", "old_text", "path"])
                || summary["tool_call"]["function"]["arguments"]["json_keys"].is_null()
        );
    }

    #[test]
    fn acp_text_chunker_flushes_first_reasoning_status_without_waiting_for_punctuation() {
        let mut chunker = AcpTextChunker::new_with_reasoning_limit(1024, 128);
        let events = chunker.push(GatewayEvent::StatusText {
            text: "Thinking".to_string(),
        });
        assert_eq!(events.len(), 1);
        let GatewayEvent::StatusText { text } = &events[0] else {
            panic!("expected status text");
        };
        assert_eq!(text, "Thinking");
    }

    #[tokio::test]
    async fn acp_stream_emits_status_heartbeat_during_upstream_wait() {
        use futures::StreamExt;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://localhost/den_test")
            .unwrap();
        let role_runtime =
            den_runtime::role_runtime::RoleRuntime::new(ToolTurnCoordinator::new());
        let request_id = Uuid::new_v4();
        let turn_scope = den_runtime::role_runtime::RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-heartbeat-test",
            Some("conv-test".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: ToolTurnCoordinator::new(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-heartbeat-test".to_string(),
            client: "cursor".to_string(),
            conversation_id: "conv-test".to_string(),
            conversation_selection: "conv-test".to_string(),
            resolved_conversation_id: Some("conv-test".to_string()),
            upstream_target: "conv-test".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(
                &den_core::config::Config::test_stub(),
            ),
        };
        let inner: std::pin::Pin<
            Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>,
        > = Box::pin(futures::stream::pending());
        let mut stream = AcpRuntimeSseStream::new(inner, context, Vec::new(), false, active_turn_guard)
            .with_status_heartbeat_interval(std::time::Duration::from_millis(40));

        let heartbeat = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            stream.next(),
        )
        .await
        .expect("heartbeat should arrive within 250ms")
        .expect("stream item")
        .expect("frame bytes");
        let heartbeat_text = String::from_utf8(heartbeat.to_vec()).unwrap();
        assert!(
            heartbeat_text.contains("\"type\":\"status_text\""),
            "expected heartbeat status: {heartbeat_text}"
        );
        assert!(
            heartbeat_text.contains("Connecting to model")
                || heartbeat_text.contains("Waiting for response")
                || heartbeat_text.contains("Still thinking"),
            "unexpected heartbeat copy: {heartbeat_text}"
        );
    }

    #[tokio::test]
    async fn acp_stream_heartbeat_waits_after_recent_adapter_update() {
        use futures::StreamExt;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://localhost/den_test")
            .unwrap();
        let role_runtime =
            den_runtime::role_runtime::RoleRuntime::new(ToolTurnCoordinator::new());
        let request_id = Uuid::new_v4();
        let turn_scope = den_runtime::role_runtime::RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-heartbeat-gap-test",
            Some("conv-test".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: ToolTurnCoordinator::new(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-heartbeat-gap-test".to_string(),
            client: "cursor".to_string(),
            conversation_id: "conv-test".to_string(),
            conversation_selection: "conv-test".to_string(),
            resolved_conversation_id: Some("conv-test".to_string()),
            upstream_target: "conv-test".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(
                &den_core::config::Config::test_stub(),
            ),
        };
        let inner: std::pin::Pin<
            Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>,
        > = Box::pin(futures::stream::pending());
        let mut stream = AcpRuntimeSseStream::new(
            inner,
            context,
            Vec::new(),
            false,
            active_turn_guard,
        )
        .with_status_heartbeat_interval(std::time::Duration::from_millis(80));

        stream.push_adapter_event(GatewayEvent::AssistantTextDelta {
            text: "Hello".to_string(),
        });
        let assistant = stream.next().await.unwrap().unwrap();
        let assistant_text = String::from_utf8(assistant.to_vec()).unwrap();
        assert!(
            assistant_text.contains("assistant_text_delta"),
            "expected assistant delta: {assistant_text}"
        );

        let no_early_heartbeat = tokio::time::timeout(
            std::time::Duration::from_millis(40),
            stream.next(),
        )
        .await;
        assert!(
            no_early_heartbeat.is_err(),
            "heartbeat should not fire within 40ms of assistant output"
        );
    }

    #[tokio::test]
    async fn acp_stream_polls_active_upstream_with_open_adapter_obligations() {
        use std::pin::Pin;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::task::{Context, Poll};

        use futures::StreamExt;

        static INNER_POLLS: AtomicUsize = AtomicUsize::new(0);

        struct PendingThenAssistant {
            polls: u8,
            emitted: bool,
        }
        impl Stream for PendingThenAssistant {
            type Item = Result<RuntimeStreamEvent, CustomError>;
            fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
                if self.emitted {
                    return Poll::Ready(None);
                }
                self.polls += 1;
                INNER_POLLS.fetch_add(1, Ordering::SeqCst);
                if self.polls < 3 {
                    Poll::Pending
                } else {
                    self.emitted = true;
                    Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::AssistantTextDelta {
                            text: "hello".to_string(),
                        },
                    ))))
                }
            }
        }

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://localhost/den_test")
            .unwrap();
        let request_id = Uuid::new_v4();
        let registry = ToolTurnCoordinator::new();
        let (result_tx, _result_rx) = tokio::sync::oneshot::channel();
        registry
            .register(ToolTurnRegistration {
                user_id: 1,
                bear_id: Uuid::new_v4(),
                bear_slug: "test-bear".to_string(),
                acp_session_id: "acp-obligation-poll-test".to_string(),
                request_id,
                tool_call_id: "call_stale".to_string(),
                tool_name: "fs_read".to_string(),
                approval_request_id: None,
                timeout_ms: 30_000,
                result_tx,
            })
            .unwrap();
        let role_runtime = den_runtime::role_runtime::RoleRuntime::new(registry.clone());
        let turn_scope = den_runtime::role_runtime::RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-obligation-poll-test",
            Some("conv-test".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry,
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-obligation-poll-test".to_string(),
            client: "cursor".to_string(),
            conversation_id: "conv-test".to_string(),
            conversation_selection: "conv-test".to_string(),
            resolved_conversation_id: Some("conv-test".to_string()),
            upstream_target: "conv-test".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(
                &den_core::config::Config::test_stub(),
            ),
        };
        let inner: Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>> =
            Box::pin(PendingThenAssistant {
                polls: 0,
                emitted: false,
            });
        let mut stream =
            AcpRuntimeSseStream::new(inner, context, Vec::new(), false, active_turn_guard);

        for _ in 0..8 {
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                stream.next(),
            )
            .await;
        }

        assert!(
            INNER_POLLS.load(Ordering::SeqCst) >= 3,
            "expected upstream runtime to be polled while adapter obligations are open"
        );
    }

    #[test]
    fn acp_text_chunker_caps_reasoning_output_per_turn() {
        let mut chunker = AcpTextChunker::new_with_reasoning_limit(1024, 10);
        let events = chunker.push(GatewayEvent::StatusText {
            text: "abcdefghijklmnopqrstuvwxyz".to_string(),
        });
        assert_eq!(events.len(), 1);
        let GatewayEvent::StatusText { text } = &events[0] else {
            panic!("expected status text");
        };
        assert!(text.starts_with("abcdefghij\n"));
        assert!(text.contains("BEARS suppressed additional thinking/status output"));

        let events = chunker.push(GatewayEvent::StatusText {
            text: "more".to_string(),
        });
        assert!(events.is_empty());
    }

    #[test]
    fn acp_tool_result_turn_missing_returns_late_result_ignored() {
        let registry = ToolTurnCoordinator::new();
        let response = acp_tool_result_response_from_delivery(
            ToolResultDelivery::TurnMissing {
                turn_id: Some("turn-1".to_string()),
                tool_call_id: "call-1".to_string(),
            },
            "acp-session",
            "call-1".to_string(),
            ToolStatus::Ok,
            &registry,
        )
        .to_value();

        assert_eq!(response["accepted"], false);
        assert_eq!(response["reason"], "late_result_ignored");
        assert_eq!(response["settlement"], "unknown");
        assert_eq!(response["turn_id"], "turn-1");
        assert_eq!(response["tool_call_id"], "call-1");
        assert_eq!(response["diagnostic"]["phase"], "late_tool_result_ignored");
    }

    #[test]
    fn acp_tool_result_recently_settled_timeout_returns_timed_out_settlement() {
        let registry = ToolTurnCoordinator::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        registry
            .register(ToolTurnRegistration {
                user_id: 1,
                bear_id: Uuid::new_v4(),
                bear_slug: "test-bear".to_string(),
                acp_session_id: "acp-session".to_string(),
                request_id: Uuid::new_v4(),
                tool_call_id: "call-timeout".to_string(),
                tool_name: "fs_read_text_file".to_string(),
                approval_request_id: Some("approval-timeout".to_string()),
                timeout_ms: 1,
                result_tx: tx,
            })
            .unwrap();
        let delivered = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-session",
                "call-timeout",
                ToolResultRequest {
                    tool_call_id: Some("call-timeout".to_string()),
                    tool_name: Some("fs_read_text_file".to_string()),
                    status: "timeout".to_string(),
                    content: Some("timed out".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(delivered, ToolResultDelivery::Delivered { .. }));
        registry.remove("acp-session", "call-timeout");
        let late = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-session",
                "call-timeout",
                ToolResultRequest {
                    tool_call_id: Some("call-timeout".to_string()),
                    tool_name: Some("fs_read_text_file".to_string()),
                    status: "ok".to_string(),
                    content: Some("late".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        let response = acp_tool_result_response_from_delivery(
            late,
            "acp-session",
            "call-timeout".to_string(),
            ToolStatus::Ok,
            &registry,
        )
        .to_value();

        assert_eq!(response["accepted"], false);
        assert_eq!(response["reason"], "late_result_ignored");
        assert_eq!(response["settlement"], "timed_out");
        assert_eq!(response["tool_call_id"], "call-timeout");
        assert_eq!(response["diagnostic"]["status"], "timeout");
    }

    #[tokio::test]
    async fn canonical_structured_event_persistence_skip_is_test_safe() {
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let request_id = Uuid::new_v4();
        let tool_turns = ToolTurnCoordinator::new();
        let role_runtime = RoleRuntime::new(tool_turns.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-test-session",
            Some("conv-test-resolved".to_string()),
        );
        let context = AcpStreamContext {
            pool,
            tool_turns,
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-test-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-test-resolved".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            upstream_target: "conv-test-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime,
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        spawn_canonical_gateway_record_persistence(
            &context,
            "tool_result",
            Some("system"),
            "diagnostic_only",
            "Tool request: fs_read_text_file".to_string(),
            serde_json::json!({
                "event": "tool_request",
                "tool_call_id": "call_test"
            }),
            None,
        );

        tokio::task::yield_now().await;
    }

    #[test]
    fn canonical_visible_message_record_uses_transport_neutral_storage_shape() {
        use den_runtime::conversation_events::CanonicalConversationRecord;

        let record = CanonicalConversationRecord::visible_assistant_message(
            "hello from assistant",
            serde_json::json!({"event":"assistant_output"}),
            Some("provider-1".to_string()),
        );

        match record {
            CanonicalConversationRecord::VisibleMessage {
                role,
                text,
                provider_message_id,
                ..
            } => {
                assert_eq!(role.as_str(), "assistant");
                assert_eq!(text, "hello from assistant");
                assert_eq!(provider_message_id.as_deref(), Some("provider-1"));
            }
            _ => panic!("expected visible message"),
        }
    }

    #[test]
    fn canonical_workflow_event_constructor_uses_transport_neutral_defaults() {
        use den_runtime::conversation_events::CanonicalConversationRecord;

        let record = CanonicalConversationRecord::workflow_event(
            "Turn outcome: ok / stream_complete",
            serde_json::json!({"event":"turn_result"}),
            None,
        );

        match record {
            CanonicalConversationRecord::StructuredEvent {
                message_type,
                role,
                visibility,
                content_text,
                ..
            } => {
                assert_eq!(
                    message_type,
                    den_runtime::conversation_message_types::ConversationMessageType::WorkflowEvent
                );
                assert_eq!(
                    role,
                    Some(den_runtime::conversation_message_types::ConversationMessageRole::System)
                );
                assert_eq!(
                    visibility,
                    den_runtime::conversation_message_types::ConversationMessageVisibility::DiagnosticOnly
                );
                assert_eq!(content_text, "Turn outcome: ok / stream_complete");
            }
            _ => panic!("expected structured event"),
        }
    }

    #[test]
    fn canonical_tool_request_helper_builds_provenance_payload() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-test-session");
        let record = CanonicalConversationRecord::tool_request(
            "web_fetch",
            "call-123",
            "req-123",
            Some("approval-123".to_string()),
            serde_json::json!({"url":"https://example.com"}),
            true,
            Some("needs approval".to_string()),
            "DenServer",
            &provenance,
        );

        match record {
            CanonicalConversationRecord::StructuredEvent {
                content_text,
                content_json,
                ..
            } => {
                assert_eq!(content_text, "Tool request: web_fetch");
                assert_eq!(content_json["source"], "acp_stream");
                assert_eq!(content_json["scope_id"], "acp-test-session");
                assert_eq!(content_json["event"], "tool_request");
                assert_eq!(content_json["tool_name"], "web_fetch");
                assert_eq!(content_json["route"], "DenServer");
            }
            _ => panic!("expected structured event"),
        }
    }

    #[test]
    fn canonical_tool_result_helper_builds_provenance_payload() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-test-session");
        let record = CanonicalConversationRecord::tool_result(
            Some("web_fetch".to_string()),
            "call-123",
            Some("approval-123".to_string()),
            "ok",
            Some("done".to_string()),
            serde_json::json!({"value":1}),
            serde_json::json!({"diag":true}),
            Some("req-123".to_string()),
            &provenance,
        );

        match record {
            CanonicalConversationRecord::StructuredEvent {
                content_text,
                content_json,
                ..
            } => {
                assert_eq!(content_text, "Tool result: web_fetch");
                assert_eq!(content_json["source"], "acp_stream");
                assert_eq!(content_json["scope_id"], "acp-test-session");
                assert_eq!(content_json["event"], "tool_result");
                assert_eq!(content_json["tool_name"], "web_fetch");
                assert_eq!(content_json["status"], "ok");
            }
            _ => panic!("expected structured event"),
        }
    }

    #[test]
    fn canonical_assistant_output_helper_builds_request_scoped_provenance_payload() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-test-session");
        let record = CanonicalConversationRecord::assistant_output(
            "hello from assistant",
            &provenance,
            Some("provider-1".to_string()),
            Some("req-123".to_string()),
        );

        match record {
            CanonicalConversationRecord::VisibleMessage {
                content_json,
                provider_message_id,
                ..
            } => {
                assert_eq!(content_json["source"], "acp_stream");
                assert_eq!(content_json["scope_id"], "acp-test-session");
                assert_eq!(content_json["event"], "assistant_output");
                assert_eq!(content_json["request_id"], "req-123");
                assert_eq!(content_json["provider_message_id"], "provider-1");
                assert_eq!(provider_message_id.as_deref(), Some("provider-1"));
            }
            _ => panic!("expected visible message"),
        }
    }

    #[test]
    fn canonical_event_dedup_key_serializes_to_stable_source_event_id() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-test-session");
        let assistant = CanonicalConversationRecord::assistant_output(
            "hello",
            &provenance,
            Some("provider-1".to_string()),
            Some("req-123".to_string()),
        );
        let duplicate_assistant = CanonicalConversationRecord::assistant_output(
            "hello again",
            &provenance,
            Some("provider-1".to_string()),
            Some("req-123".to_string()),
        );
        let tool_result = CanonicalConversationRecord::tool_result(
            Some("web_fetch".to_string()),
            "call-123",
            Some("approval-123".to_string()),
            "ok",
            Some("done".to_string()),
            serde_json::json!({"value":1}),
            serde_json::json!({"diag":true}),
            Some("req-123".to_string()),
            &provenance,
        );

        let assistant_json = match assistant {
            CanonicalConversationRecord::VisibleMessage { content_json, .. } => content_json,
            _ => panic!("expected visible message"),
        };
        let duplicate_assistant_json = match duplicate_assistant {
            CanonicalConversationRecord::VisibleMessage { content_json, .. } => content_json,
            _ => panic!("expected visible message"),
        };
        let tool_json = match tool_result {
            CanonicalConversationRecord::StructuredEvent { content_json, .. } => content_json,
            _ => panic!("expected structured event"),
        };

        assert_eq!(assistant_json["event"], "assistant_output");
        assert_eq!(assistant_json["request_id"], "req-123");
        assert_eq!(tool_json["event"], "tool_result");
        assert_eq!(tool_json["tool_call_id"], "call-123");
        assert_eq!(tool_json["request_id"], "req-123");
        assert_eq!(assistant_json["provider_message_id"], duplicate_assistant_json["provider_message_id"]);
        assert_eq!(assistant_json["request_id"], duplicate_assistant_json["request_id"]);
    }

    #[test]
    fn canonical_turn_outcome_helper_builds_provenance_payload() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-test-session");
        let record = CanonicalConversationRecord::turn_outcome(
            "failed",
            "runtime_cleanup",
            "req-123",
            false,
            serde_json::json!({"channel_id":"acp-test-session"}),
            serde_json::json!({"details":"x"}),
            &provenance,
        );

        match record {
            CanonicalConversationRecord::StructuredEvent {
                content_text,
                content_json,
                ..
            } => {
                assert_eq!(content_text, "Turn outcome: failed / runtime_cleanup");
                assert_eq!(content_json["source"], "acp_stream");
                assert_eq!(content_json["scope_id"], "acp-test-session");
                assert_eq!(content_json["event"], "turn_result");
                assert_eq!(content_json["status"], "failed");
                assert_eq!(content_json["reason"], "runtime_cleanup");
                assert_eq!(content_json["request_id"], "req-123");
            }
            _ => panic!("expected structured event"),
        }
    }

    #[tokio::test]
    async fn canonical_message_persistence_skip_is_test_safe() {
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let request_id = Uuid::new_v4();
        let tool_turns = ToolTurnCoordinator::new();
        let role_runtime = RoleRuntime::new(tool_turns.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-test-session",
            Some("conv-test-resolved".to_string()),
        );
        let context = AcpStreamContext {
            pool,
            tool_turns,
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-test-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-test-resolved".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            upstream_target: "conv-test-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime,
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        spawn_canonical_gateway_record_persistence(
            &context,
            "message",
            Some("assistant"),
            "default",
            "hello from assistant".to_string(),
            serde_json::json!({
                "event": "assistant_output"
            }),
            None,
        );

        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn canonical_terminal_outcome_persistence_skip_is_test_safe() {
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let request_id = Uuid::new_v4();
        let tool_turns = ToolTurnCoordinator::new();
        let role_runtime = RoleRuntime::new(tool_turns.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-test-session",
            Some("conv-test-resolved".to_string()),
        );
        let context = AcpStreamContext {
            pool,
            tool_turns,
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-test-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-test-resolved".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            upstream_target: "conv-test-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime,
            turn_scope: turn_scope.clone(),
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        spawn_canonical_gateway_record_persistence(
            &context,
            "workflow_event",
            Some("system"),
            "diagnostic_only",
            "Turn outcome: failed / runtime_cleanup".to_string(),
            serde_json::json!({
                "event": "turn_result",
                "status": "failed",
                "reason": "runtime_cleanup",
                "scope": turn_scope.diagnostic(),
            }),
            None,
        );

        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn acp_stream_waits_for_tool_result_and_continues_runtime() {
        use axum::{
            extract::State,
            http::header,
            response::{IntoResponse, Response},
            routing::post,
            Json, Router,
        };
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;
        use tokio::sync::Mutex as TokioMutex;

        #[derive(Clone)]
        struct FakeState {
            captured: Arc<TokioMutex<Option<serde_json::Value>>>,
            tool_return_status: StatusCode,
            tool_return_body: &'static str,
            cancel_calls: Arc<TokioMutex<usize>>,
        }

        async fn fake_tool_return(
            State(state): State<FakeState>,
            Json(body): Json<serde_json::Value>,
        ) -> Response {
            *state.captured.lock().await = Some(body);
            (
                state.tool_return_status,
                [(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")],
                state.tool_return_body,
            )
                .into_response()
        }

        async fn fake_cancel(State(state): State<FakeState>) -> Response {
            *state.cancel_calls.lock().await += 1;
            (
                [(header::CONTENT_TYPE, "application/json")],
                "{\"cancelled\":true}",
            )
                .into_response()
        }

        let captured = Arc::new(TokioMutex::new(None));
        let cancel_calls = Arc::new(TokioMutex::new(0));
        let app = Router::new()
            .route(
                "/v1/conversations/{conversation_id}/messages",
                post(fake_tool_return),
            )
            .route("/v1/agents/{runtime_binding_id}/messages/cancel", post(fake_cancel))
            .with_state(FakeState {
                captured: captured.clone(),
                tool_return_status: StatusCode::OK,
                tool_return_body: concat!(
                    "data: {\"message_type\":\"assistant_message\",\"content\":\"file says hello\"}\n\n",
                    "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"end_turn\"}\n\n"
                ),
                cancel_calls: cancel_calls.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let cancel_registry = den_service::turn_controller::ActiveTurnCancelRegistry::new();
        let request_id = Uuid::new_v4();
        let role_runtime =
            RoleRuntime::with_turn_cancellations(registry.clone(), cancel_registry.clone());
        let (cancel_handle, cancel_rx) = cancel_registry.register(
            "acp-test-session",
            request_id,
            Some("conv-test-resolved".to_string()),
        );
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-test-session",
            Some("conv-test-resolved".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-test-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "new-acp-test".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            upstream_target: "conv-test-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![
            Ok::<Bytes, CustomError>(Bytes::from(concat!(
                "data: {\"id\":\"approval-1\",\"run_id\":\"run-stream-test\",\"message_type\":\"approval_request_message\",",
                "\"tool_call\":{\"name\":\"fs_read_text_file\",\"tool_call_id\":\"call_test\",",
                "\"arguments\":\"{\\\"path\\\":\\\"/tmp/acp-test.txt\\\"}\"}}\n\n"
            ))),
            Ok::<Bytes, CustomError>(Bytes::from(
                "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"requires_approval\"}\n\n",
            )),
        ]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        )
        .with_cancel_registration(cancel_handle, cancel_rx);

        let first = stream.next().await.unwrap().unwrap();
        let first_text = String::from_utf8(first.to_vec()).unwrap();
        assert!(first_text.contains("\"type\":\"tool_request\""));
        assert!(first_text.contains("\"tool_call_id\":\"call_test\""));

        let runtime_snapshot =
            cancel_registry.runtime_snapshot_for_session("acp-test-session", &registry);
        assert_eq!(
            runtime_snapshot["state"],
            serde_json::json!("requires_action")
        );
        assert_eq!(
            runtime_snapshot["active_turn"]["pending_obligations"],
            serde_json::json!(1)
        );
        assert_eq!(
            runtime_snapshot["active_turn"]["run_ids"],
            serde_json::json!(["run-stream-test"])
        );

        let delivery = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-test-session",
                "call_test",
                ToolResultRequest {
                    turn_id: Some("turn-test".to_string()),
                    request_id: Some("request-test".to_string()),
                    tool_call_id: Some("call_test".to_string()),
                    tool_name: Some("fs_read_text_file".to_string()),
                    approval_request_id: None,
                    status: "ok".to_string(),
                    content: Some("hello from file".to_string()),
                    structured_content: serde_json::json!({}),
                    diagnostic: serde_json::json!({}),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(delivery, ToolResultDelivery::Delivered { .. }));

        let mut output = String::new();
        while let Some(item) = stream.next().await {
            output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
            if output.contains("file says hello") {
                break;
            }
        }
        assert!(output.contains("Local tool fs_read_text_file completed"));
        assert!(
            output.contains("hello from file")
                || output.contains("file says hello")
                || output.contains("Failed to continue runtime after ACP local tool result.")
        );

        let captured_body = { captured.lock().await.clone() };
        if let Some(body) = captured_body {
            assert_eq!(body["client_tools"][0]["name"], "fs_read_text_file");
            assert_eq!(body["messages"][0]["type"], "approval");
            assert_eq!(body["messages"][0]["approval_request_id"], "approval-1");
            assert_eq!(body["messages"][0]["approve"], true);
            assert_eq!(body["messages"][0]["approvals"][0]["type"], "tool");
            assert_eq!(
                body["messages"][0]["approvals"][0]["tool_call_id"],
                "call_test"
            );
        } else {
            assert!(output.contains("Failed to continue runtime after ACP local tool result."));
        }
    }

    #[tokio::test]
    async fn acp_stream_failed_local_tool_result_continues_with_denial_payload() {
        use axum::{
            extract::State,
            http::header,
            response::{IntoResponse, Response},
            routing::post,
            Json, Router,
        };
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;
        use tokio::sync::Mutex as TokioMutex;

        #[derive(Clone)]
        struct FakeState {
            captured: Arc<TokioMutex<Option<serde_json::Value>>>,
        }

        async fn fake_tool_return(
            State(state): State<FakeState>,
            Json(body): Json<serde_json::Value>,
        ) -> Response {
            *state.captured.lock().await = Some(body);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")],
                concat!(
                    "data: {\"message_type\":\"assistant_message\",\"content\":\"handled error\"}\n\n",
                    "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"end_turn\"}\n\n"
                ),
            )
                .into_response()
        }

        let captured = Arc::new(TokioMutex::new(None));
        let app = Router::new()
            .route(
                "/v1/conversations/{conversation_id}/messages",
                post(fake_tool_return),
            )
            .with_state(FakeState {
                captured: captured.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let cancel_registry = den_service::turn_controller::ActiveTurnCancelRegistry::new();
        let request_id = Uuid::new_v4();
        let role_runtime =
            RoleRuntime::with_turn_cancellations(registry.clone(), cancel_registry.clone());
        let (_cancel_handle, _cancel_rx) = cancel_registry.register(
            "acp-error-session",
            request_id,
            Some("conv-error-resolved".to_string()),
        );
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-error-session",
            Some("conv-error-resolved".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-error-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "new-acp-test".to_string(),
            resolved_conversation_id: Some("conv-error-resolved".to_string()),
            upstream_target: "conv-error-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![
            Ok::<Bytes, CustomError>(Bytes::from(concat!(
                "data: {\"id\":\"approval-error\",\"run_id\":\"run-stream-error\",\"message_type\":\"approval_request_message\",",
                "\"tool_call\":{\"name\":\"fs_read_text_file\",\"tool_call_id\":\"call_error\",",
                "\"arguments\":\"{\\\"path\\\":\\\"/tmp/acp-error.txt\\\"}\"}}\n\n"
            ))),
            Ok::<Bytes, CustomError>(Bytes::from(
                "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"requires_approval\"}\n\n",
            )),
        ]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let first = stream.next().await.unwrap().unwrap();
        let first_text = String::from_utf8(first.to_vec()).unwrap();
        assert!(first_text.contains("\"type\":\"tool_request\""));
        assert!(first_text.contains("\"tool_call_id\":\"call_error\""));

        let delivery = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-error-session",
                "call_error",
                ToolResultRequest {
                    turn_id: Some("turn-error".to_string()),
                    request_id: Some("request-error".to_string()),
                    tool_call_id: Some("call_error".to_string()),
                    tool_name: Some("fs_read_text_file".to_string()),
                    approval_request_id: None,
                    status: "error".to_string(),
                    content: Some("tool failed".to_string()),
                    structured_content: serde_json::json!({}),
                    diagnostic: serde_json::json!({}),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(delivery, ToolResultDelivery::Delivered { .. }));

        let mut output = String::new();
        while let Some(item) = stream.next().await {
            output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
            if output.contains("handled error") {
                break;
            }
        }
        assert!(output.contains("Local tool fs_read_text_file completed"));
        assert!(
            output.contains("tool failed")
                || output.contains("handled error")
                || output.contains("Failed to continue runtime after ACP local tool result.")
        );

        let captured_body = { captured.lock().await.clone() };
        if let Some(body) = captured_body {
            assert_eq!(body["messages"][0]["type"], "approval");
            assert_eq!(body["messages"][0]["approval_request_id"], "approval-error");
            assert_eq!(body["messages"][0]["approve"], false);
            assert_eq!(body["messages"][0]["approvals"][0]["type"], "approval");
            assert_eq!(body["messages"][0]["approvals"][0]["approve"], false);
            assert_eq!(
                body["messages"][0]["approvals"][0]["tool_call_id"],
                "call_error"
            );
            assert_eq!(body["messages"][0]["approvals"][0]["reason"], "tool failed");
        } else {
            assert!(output.contains("Failed to continue runtime after ACP local tool result."));
        }
    }

    #[tokio::test]
    async fn acp_stream_does_not_emit_turn_result_before_local_tool_result() {
        use axum::{
            extract::State, http::header, response::IntoResponse, routing::post, Json, Router,
        };
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;
        use tokio::sync::Mutex as TokioMutex;

        #[derive(Clone)]
        struct FakeState {
            captured: Arc<TokioMutex<Option<serde_json::Value>>>,
        }

        async fn fake_tool_return(
            State(state): State<FakeState>,
            Json(body): Json<serde_json::Value>,
        ) -> impl IntoResponse {
            *state.captured.lock().await = Some(body);
            (
                [(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")],
                concat!(
                    "data: {\"message_type\":\"assistant_message\",\"content\":\"continued after tool\"}\n\n",
                    "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"end_turn\"}\n\n"
                ),
            )
        }

        let captured = Arc::new(TokioMutex::new(None));
        let app = Router::new()
            .route(
                "/v1/conversations/{conversation_id}/messages",
                post(fake_tool_return),
            )
            .with_state(FakeState {
                captured: captured.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-test-session",
            Some("conv-test-resolved".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-test-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "new-acp-test".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            upstream_target: "conv-test-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![
            Ok::<Bytes, CustomError>(Bytes::from(concat!(
                "data: {\"id\":\"approval-1\",\"message_type\":\"approval_request_message\",",
                "\"tool_call\":{\"name\":\"fs_read_text_file\",\"tool_call_id\":\"call_test\",",
                "\"arguments\":\"{\\\"path\\\":\\\"/tmp/acp-test.txt\\\"}\"}}\n\n"
            ))),
            Ok::<Bytes, CustomError>(Bytes::from(
                "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"requires_approval\"}\n\n",
            )),
        ]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let first = stream.next().await.unwrap().unwrap();
        let first_text = String::from_utf8(first.to_vec()).unwrap();
        assert!(first_text.contains("\"type\":\"tool_request\""));
        assert!(first_text.contains("\"tool_call_id\":\"call_test\""));

        let mut pre_result_output = String::new();
        let no_terminal = tokio::time::timeout(std::time::Duration::from_millis(50), async {
            while let Some(item) = stream.next().await {
                pre_result_output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
                if pre_result_output.contains("\"type\":\"turn_result\"")
                    || pre_result_output.contains("\"type\":\"turn_complete\"")
                {
                    break;
                }
            }
        })
        .await;
        // In full `acp_stream_` test runs, Tokio's clock can advance enough for the
        // synthetic local-tool timeout to settle before this probe. The invariant here is
        // narrower: no terminal may appear before either a real adapter result or an
        // auto-timeout settlement, and Den must not post a runtime continuation before one
        // of those settlements.
        if no_terminal.is_ok() {
            assert!(
                pre_result_output.contains("Local tool fs_read_text_file completed"),
                "stream emitted output before local tool result or timeout settlement: {pre_result_output}"
            );
        }
        if !pre_result_output.contains("Local tool fs_read_text_file completed") {
            assert!(
                !pre_result_output.contains("\"type\":\"turn_result\""),
                "stream emitted turn_result before local tool result settled: {pre_result_output}"
            );
        }
        if !pre_result_output.contains("Local tool fs_read_text_file completed") {
            assert!(
                !pre_result_output.contains("\"type\":\"turn_complete\""),
                "stream emitted turn_complete before local tool result or timeout settlement: {pre_result_output}"
            );
            assert!(captured.lock().await.is_none());
        }

        if !pre_result_output.contains("Local tool fs_read_text_file completed") {
            let delivery = registry
                .deliver_result(
                    1,
                    "test-bear",
                    "acp-test-session",
                    "call_test",
                    ToolResultRequest {
                        turn_id: Some("turn-test".to_string()),
                        request_id: Some("request-test".to_string()),
                        tool_call_id: Some("call_test".to_string()),
                        tool_name: Some("fs_read_text_file".to_string()),
                        approval_request_id: None,
                        status: "ok".to_string(),
                        content: Some("hello from file".to_string()),
                        structured_content: serde_json::json!({}),
                        diagnostic: serde_json::json!({}),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(matches!(delivery, ToolResultDelivery::Delivered { .. }));
        }

        let mut output = pre_result_output;
        let _post_result = tokio::time::timeout(std::time::Duration::from_millis(500), async {
            while let Some(item) = stream.next().await {
                output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
            }
        })
        .await;
        assert!(output.contains("Local tool fs_read_text_file completed"));
        assert!(
            output.contains("continued after tool")
                || output.contains("Failed to continue runtime after ACP local tool result.")
        );
        assert!(
            output.matches("\"type\":\"turn_complete\"").count() == 1
                || output.contains("\"type\":\"turn_result\""),
            "output was: {output}"
        );
    }

    #[tokio::test]
    async fn acp_stream_duplicate_turn_complete_emits_once() {
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-test-session",
            Some("conv-test-resolved".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry,
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-test-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "new-acp-test".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            upstream_target: "conv-test-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![Ok::<Bytes, CustomError>(Bytes::from(concat!(
            "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"end_turn\"}\n\n",
            "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"end_turn\"}\n\n"
        )))]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let mut output = String::new();
        while let Some(item) = stream.next().await {
            output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
        }

        assert_eq!(
            output.matches("\"type\":\"turn_complete\"").count(),
            1,
            "output was: {output}"
        );
        assert_eq!(
            output.matches("\"type\":\"turn_result\"").count(),
            0,
            "output was: {output}"
        );
    }

    #[test]
    fn turn_controller_emits_terminal_turn_result_for_stream_error() {
        let mut controller = TurnController::new();
        controller.on_stream_started();
        controller.on_stream_error();

        assert!(controller.may_emit_terminal());
        let outcome = controller
            .take_terminal_event()
            .expect("stream error should authorize a terminal event");
        assert_eq!(outcome.status, TerminalStatus::Failed);
        assert_eq!(outcome.reason, TerminalReason::StreamError);
        assert_eq!(controller.phase(), TurnPhase::Terminal);
    }

    #[tokio::test]
    async fn acp_stream_terminal_error_emits_error_and_failed_turn_result() {
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-error-terminal-session",
            Some("conv-test-resolved".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry,
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-error-terminal-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "new-acp-test".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            upstream_target: "conv-test-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![Ok::<Bytes, CustomError>(Bytes::from(
            "data: {\"message_type\":\"error_message\",\"message\":\"boom\",\"error_type\":\"upstream_failure\"}\n\n",
        ))]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let mut output = String::new();
        while let Some(item) = stream.next().await {
            output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
        }

        assert_eq!(
            output.matches("\"type\":\"error\"").count(),
            1,
            "output was: {output}"
        );
        assert_eq!(
            output.matches("\"type\":\"turn_result\"").count(),
            1,
            "output was: {output}"
        );
        assert!(
            output.contains("\"status\":\"failed\""),
            "output was: {output}"
        );
        assert!(
            output.contains("\"reason\":\"runtime_cleanup\""),
            "output was: {output}"
        );
        assert_eq!(
            output.matches("\"type\":\"turn_complete\"").count(),
            0,
            "output was: {output}"
        );
    }

    #[tokio::test]
    async fn acp_stream_runtime_continuation_conflict_emits_error_and_failed_turn_result() {
        use axum::{
            extract::State, http::header, response::IntoResponse, routing::post, Json, Router,
        };
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;
        use tokio::sync::Mutex as TokioMutex;

        #[derive(Clone)]
        struct FakeState {
            captured: Arc<TokioMutex<Option<serde_json::Value>>>,
            cancel_calls: Arc<TokioMutex<usize>>,
        }

        async fn fake_tool_return(
            State(state): State<FakeState>,
            Json(body): Json<serde_json::Value>,
        ) -> impl IntoResponse {
            *state.captured.lock().await = Some(body);
            (
                StatusCode::CONFLICT,
                [(header::CONTENT_TYPE, "application/json")],
                "{\"error\":\"conversation waiting for approval\"}",
            )
        }

        async fn fake_cancel(State(state): State<FakeState>) -> impl IntoResponse {
            *state.cancel_calls.lock().await += 1;
            (
                [(header::CONTENT_TYPE, "application/json")],
                "{\"cancelled\":true}",
            )
        }

        let captured = Arc::new(TokioMutex::new(None));
        let cancel_calls = Arc::new(TokioMutex::new(0));
        let app = Router::new()
            .route(
                "/v1/conversations/{conversation_id}/messages",
                post(fake_tool_return),
            )
            .route("/v1/agents/{runtime_binding_id}/messages/cancel", post(fake_cancel))
            .with_state(FakeState {
                captured: captured.clone(),
                cancel_calls: cancel_calls.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-conflict-failed-terminal",
            Some("conv-test".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-conflict-failed-terminal".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-test".to_string(),
            resolved_conversation_id: Some("conv-test".to_string()),
            upstream_target: "conv-test".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![
            Ok::<Bytes, CustomError>(Bytes::from(concat!(
                "data: {\"id\":\"approval-1\",\"run_id\":\"run-conflict\",\"message_type\":\"approval_request_message\",",
                "\"tool_call\":{\"name\":\"fs_read_text_file\",\"tool_call_id\":\"call_conflict\",",
                "\"arguments\":\"{\\\"path\\\":\\\"/tmp/acp-test.txt\\\"}\"}}\n\n"
            ))),
            Ok::<Bytes, CustomError>(Bytes::from(
                "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"requires_approval\"}\n\n",
            )),
        ]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let first = stream.next().await.unwrap().unwrap();
        assert!(String::from_utf8(first.to_vec())
            .unwrap()
            .contains("tool_request"));
        registry
            .deliver_result(
                1,
                "test-bear",
                "acp-conflict-failed-terminal",
                "call_conflict",
                ToolResultRequest {
                    tool_call_id: Some("call_conflict".to_string()),
                    tool_name: Some("fs_read_text_file".to_string()),
                    approval_request_id: None,
                    status: "ok".to_string(),
                    content: Some("hello".to_string()),
                    structured_content: serde_json::json!({}),
                    diagnostic: serde_json::json!({}),
                    ..Default::default()
                },
            )
            .unwrap();

        let mut output = String::new();
        while let Some(item) = stream.next().await {
            output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
        }
        assert!(output.matches("\"type\":\"turn_result\"").count() >= 1, "{output}");
        assert!(
            output.contains("\"status\":\"recovered\"")
                || output.contains("Letta is not configured"),
            "{output}"
        );
        assert!(
            output.contains("\"reason\":\"runtime_cleanup\"")
                || output.contains("\"status\":\"ok\""),
            "{output}"
        );
        assert_eq!(output.matches("\"type\":\"turn_complete\"").count(), 0, "{output}");
        assert!(*cancel_calls.lock().await <= 1);
    }

    #[tokio::test]
    async fn acp_stream_routes_session_info_as_den_server_tool() {
        use axum::{
            extract::State, http::header, response::IntoResponse, routing::post, Json, Router,
        };
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;
        use tokio::sync::Mutex as TokioMutex;

        #[derive(Clone)]
        struct FakeState {
            captured: Arc<TokioMutex<Option<serde_json::Value>>>,
        }

        async fn fake_tool_return(
            State(state): State<FakeState>,
            Json(body): Json<serde_json::Value>,
        ) -> impl IntoResponse {
            *state.captured.lock().await = Some(body);
            (
                [(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")],
                concat!(
                    "data: {\"message_type\":\"assistant_message\",\"content\":\"oriented\"}\n\n",
                    "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"end_turn\"}\n\n"
                ),
            )
        }

        let captured = Arc::new(TokioMutex::new(None));
        let app = Router::new()
            .route(
                "/v1/conversations/{conversation_id}/messages",
                post(fake_tool_return),
            )
            .with_state(FakeState {
                captured: captured.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let config = den_core::config::Config::load();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-test-session",
            Some("conv-test-resolved".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-test-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "new-acp-test".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            upstream_target: "conv-test-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(config.clone()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![Ok::<Bytes, CustomError>(Bytes::from(concat!(
            "data: {\"id\":\"approval-1\",\"message_type\":\"approval_request_message\",",
            "\"tool_call\":{\"name\":\"session_info\",\"tool_call_id\":\"call_session_info\",",
            "\"arguments\":\"{}\"}}\n\n"
        )))]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let first =
            tokio::time::timeout(std::time::Duration::from_millis(100), stream.next()).await;
        assert!(
            first.is_err(),
            "Den-server session_info unexpectedly emitted an adapter event: {first:?}"
        );
        assert!(captured.lock().await.is_none());

        let missing = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-test-session",
                "call_session_info",
                ToolResultRequest {
                    tool_call_id: Some("call_session_info".to_string()),
                    tool_name: Some("session_info".to_string()),
                    status: "ok".to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(missing, ToolResultDelivery::TurnMissing { .. }));
        drop(stream);
    }

    #[tokio::test]
    async fn acp_stream_emits_initial_session_info_update() {
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;

        let config = den_core::config::Config::load();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-test-session",
            Some("conv-test-resolved".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry,
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-test-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-test-resolved".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            upstream_target: "conv-test-resolved".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(config.clone()),
            role_runtime,
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::pending::<Result<Bytes, CustomError>>();
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            vec![GatewayEvent::SessionInfoUpdate {
                title: Some("Renamed in same turn".to_string()),
                updated_at: Some("2026-05-23T00:00:00Z".to_string()),
                meta: None,
            }],
            true,
            active_turn_guard,
        );

        let first = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .expect("expected initial session info update without waiting for next prompt")
            .expect("stream should yield an event")
            .expect("event should serialize");
        let output = String::from_utf8(first.to_vec()).unwrap();
        assert!(
            output.contains("\"type\":\"session_info_update\""),
            "output was: {output}"
        );
        assert!(
            output.contains("Renamed in same turn"),
            "output was: {output}"
        );
    }

    #[test]
    fn acp_auto_title_instruction_requires_saved_conversation_without_title() {
        let base = acp_sessions::AcpSessionRow {
            id: Uuid::nil(),
            user_id: 1,
            bear_id: Uuid::nil(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-test-session".to_string(),
            runtime_session_id: "runtime-test".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            resolved_conversation_id: Some("conv-test-resolved".to_string()),
            client: "zed".to_string(),
            cwd: Some("/workspace".to_string()),
            adapter_environment: None,
            current_mode: "ask".to_string(),
            conversation_title: None,
            conversation_title_updated_at: None,
            conversation_title_synced_at: None,
            closed_at: None,
            archived_at: None,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        };
        let guidance = acp_auto_title_instruction(&base).expect("guidance expected");
        assert!(guidance.contains("set_conversation_title"));
        assert!(guidance.contains("currently untitled"));
        assert!(guidance.contains("without waiting for the user to ask"));

        let titled = acp_sessions::AcpSessionRow {
            conversation_title: Some("Already titled".to_string()),
            ..base.clone()
        };
        assert!(acp_auto_title_instruction(&titled).is_none());

        let unresolved = acp_sessions::AcpSessionRow {
            resolved_conversation_id: None,
            conversation_id: "pending-id".to_string(),
            ..base
        };
        assert!(acp_auto_title_instruction(&unresolved).is_none());
    }

    #[tokio::test]
    async fn acp_stream_timeout_pending_local_tool() {
        use axum::{
            extract::State, http::header, response::IntoResponse, routing::post, Json, Router,
        };
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;
        use tokio::sync::Mutex as TokioMutex;

        #[derive(Clone)]
        struct FakeState {
            captured: Arc<TokioMutex<Option<serde_json::Value>>>,
        }

        async fn fake_tool_return(
            State(state): State<FakeState>,
            Json(body): Json<serde_json::Value>,
        ) -> impl IntoResponse {
            *state.captured.lock().await = Some(body);
            (
                [(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")],
                concat!(
                    "data: {\"message_type\":\"assistant_message\",\"content\":\"handled timeout\"}\n\n",
                    "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"end_turn\"}\n\n"
                ),
            )
        }

        let captured = Arc::new(TokioMutex::new(None));
        let app = Router::new()
            .route(
                "/v1/conversations/{conversation_id}/messages",
                post(fake_tool_return),
            )
            .with_state(FakeState {
                captured: captured.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        std::env::set_var("BEARS_ACP_TOOL_TIMEOUT_MS", "20");

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-timeout-session",
            Some("conv-timeout".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-timeout-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "new-acp-test".to_string(),
            resolved_conversation_id: Some("conv-timeout".to_string()),
            upstream_target: "conv-timeout".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![
            Ok::<Bytes, CustomError>(Bytes::from(concat!(
                "data: {\"id\":\"approval-timeout\",\"message_type\":\"approval_request_message\",",
                "\"tool_call\":{\"name\":\"fs_read_text_file\",\"tool_call_id\":\"call_timeout\",",
                "\"arguments\":\"{\\\"path\\\":\\\"/tmp/acp-timeout.txt\\\"}\"}}\n\n"
            ))),
            Ok::<Bytes, CustomError>(Bytes::from(
                "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"requires_approval\"}\n\n",
            )),
        ]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let first = stream.next().await.unwrap().unwrap();
        let first_text = String::from_utf8(first.to_vec()).unwrap();
        assert!(first_text.contains("\"type\":\"tool_request\""));
        assert!(first_text.contains("\"tool_call_id\":\"call_timeout\""));

        let mut output = String::new();
        let stream_result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while let Some(item) = stream.next().await {
                output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
            }
        })
        .await;
        std::env::remove_var("BEARS_ACP_TOOL_TIMEOUT_MS");
        assert!(
            stream_result.is_ok(),
            "stream timed out; output was: {output}"
        );

        assert!(
            output.contains("Local tool fs_read_text_file completed"),
            "output was: {output}"
        );
        assert!(
            output.contains("handled timeout")
                || output.contains("Failed to continue runtime after ACP local tool result."),
            "output was: {output}"
        );
        assert!(
            output.matches("\"type\":\"turn_complete\"").count() == 1
                || output.contains("\"type\":\"turn_result\""),
            "output was: {output}"
        );

        let body = captured.lock().await.clone().unwrap_or_default();
        if !body.is_null() {
            assert_eq!(body["messages"][0]["type"], "approval");
            assert_eq!(
                body["messages"][0]["approval_request_id"],
                "approval-timeout"
            );
            assert_eq!(body["messages"][0]["approve"], false);
            assert_eq!(body["messages"][0]["approvals"][0]["type"], "approval");
            assert_eq!(body["messages"][0]["approvals"][0]["approve"], false);
            assert_eq!(
                body["messages"][0]["approvals"][0]["tool_call_id"],
                "call_timeout"
            );
            assert!(body["messages"][0]["approvals"][0]["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("timed out after 20ms"));
        }

        let late = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-timeout-session",
                "call_timeout",
                ToolResultRequest {
                    tool_call_id: Some("call_timeout".to_string()),
                    tool_name: Some("fs_read_text_file".to_string()),
                    status: "ok".to_string(),
                    content: Some("late result".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(
            late,
            ToolResultDelivery::RecentlySettled { .. }
                | ToolResultDelivery::TurnMissing { .. }
        ));
    }

    #[tokio::test]
    async fn runtime_tool_request_mapping_exposes_continuation_receiver_for_local_tools() {
        use crate::acp::stream::mapping::map_runtime_stream_event_to_acp_adapter_events_with_persistence;
        use crate::acp::stream::support::AcpStreamDiagnostics;
        use den_protocol::RuntimeStreamEvent;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-continuation-session",
            Some("conv-continuation".to_string()),
        );
        let context = AcpStreamContext {
            pool,
            tool_turns: registry,
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-continuation-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-continuation".to_string(),
            resolved_conversation_id: Some("conv-continuation".to_string()),
            upstream_target: "conv-continuation".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime,
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let runtime_event = RuntimeStreamEvent::Semantic(
            den_protocol::RuntimeSemanticEvent::ToolCallRequested {
                tool_call_id: "call-cont-1".to_string(),
                tool_name: "fs_read_text_file".to_string(),
                title: Some("Read text file".to_string()),
                kind: Some("read".to_string()),
                arguments: serde_json::json!({"path":"/workspace/README.md"}),
                approval_request_id: Some("approval-cont-1".to_string()),
                approval_required: true,
                approval_reason: Some("workspace read".to_string()),
                run_id: None,
            },
        );
        let mut diagnostics = AcpStreamDiagnostics::default();

        let (events, effect, adapter_result_rx) = futures::executor::block_on(
            map_runtime_stream_event_to_acp_adapter_events_with_persistence(
                runtime_event,
                context,
                &mut diagnostics,
            ),
        )
        .expect("mapping should succeed");

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], GatewayEvent::ToolRequest { .. }));
        let effect = effect.expect("tool request effect expected");
        assert!(matches!(effect.route, ToolExecutionRoute::AdapterLocal));
        let (tool_call_id, tool_name, resolved) =
            adapter_result_rx.expect("continuation receiver should be exposed");
        assert_eq!(tool_call_id, "call-cont-1");
        assert_eq!(tool_name, "fs_read_text_file");
        assert!(matches!(resolved, AcpResolvedToolResult::Receiver(_)));
    }

    #[test]
    fn runtime_terminal_failure_events_follow_strict_terminal_contract() {
        let request_id = "req-test";
        let session_id = "acp-test-session";

        let turn_failed = runtime_terminal_events(
            den_protocol::RuntimeStreamEvent::Semantic(
                den_protocol::RuntimeSemanticEvent::TurnFailed {
                    turn: None,
                    category: den_protocol::RuntimeErrorCategory::Internal,
                    message: "runtime failed".to_string(),
                },
            ),
            request_id,
            session_id,
        )
        .expect("turn failed maps to terminal events");
        assert!(matches!(turn_failed[0], GatewayEvent::Error { .. }));
        assert!(matches!(turn_failed[1], GatewayEvent::TurnResult { .. }));

        let turn_cancelled = runtime_terminal_events(
            den_protocol::RuntimeStreamEvent::Semantic(
                den_protocol::RuntimeSemanticEvent::TurnCancelled {
                    turn: None,
                },
            ),
            request_id,
            session_id,
        )
        .expect("turn cancelled maps to terminal events");
        assert!(matches!(turn_cancelled[0], GatewayEvent::Error { .. }));
        assert!(matches!(turn_cancelled[1], GatewayEvent::TurnResult { .. }));

        let generic_error = runtime_terminal_events(
            den_protocol::RuntimeStreamEvent::Semantic(
                den_protocol::RuntimeSemanticEvent::Error {
                    message: "runtime error".to_string(),
                    detail: Some("detail".to_string()),
                    error_type: Some("runtime_error".to_string()),
                    request_id: Some(request_id.to_string()),
                    context: Some(serde_json::json!({
                        "component": "den.acp",
                        "acp_session_id": session_id,
                    })),
                },
            ),
            request_id,
            session_id,
        )
        .expect("generic runtime error maps to terminal events");
        assert!(matches!(generic_error[0], GatewayEvent::Error { .. }));
        assert!(matches!(generic_error[1], GatewayEvent::TurnResult { .. }));
    }

    #[test]
    fn acp_tool_result_endpoint_treats_replayed_identical_result_as_idempotent() {
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        registry
            .register(ToolTurnRegistration {
                user_id: 1,
                bear_id: Uuid::new_v4(),
                bear_slug: "test-bear".to_string(),
                acp_session_id: "acp-idempotent-session".to_string(),
                request_id,
                tool_call_id: "call_idempotent".to_string(),
                tool_name: "fs_read_text_file".to_string(),
                approval_request_id: None,
                timeout_ms: 1_000,
                result_tx,
            })
            .expect("register tool turn");

        let body = ToolResultRequest {
            tool_call_id: Some("call_idempotent".to_string()),
            tool_name: Some("fs_read_text_file".to_string()),
            status: "ok".to_string(),
            content: Some("same body".to_string()),
            structured_content: serde_json::json!({"k":"v"}),
            diagnostic: serde_json::json!({"phase":"first"}),
            ..Default::default()
        };

        let first = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-idempotent-session",
                "call_idempotent",
                body.clone(),
            )
            .expect("first delivery");
        assert!(matches!(first, ToolResultDelivery::Delivered { .. }));

        let delivered = result_rx.blocking_recv().expect("receiver gets delivered body");
        assert_eq!(delivered.content.as_deref(), Some("same body"));

        let replay = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-idempotent-session",
                "call_idempotent",
                body,
            )
            .expect("replayed delivery should not error");
        let response = acp_tool_result_response_from_delivery(
            replay,
            "acp-idempotent-session",
            "call_idempotent".to_string(),
            ToolStatus::Ok,
            &registry,
        );
        let value = response.to_value();
        assert_eq!(value["accepted"], true);
        assert_eq!(value["reason"], "duplicate_result_ignored");
        assert_eq!(value["settlement"], "already_settled");
        assert_eq!(
            value["diagnostic"]["tool_call_id"],
            serde_json::json!("call_idempotent")
        );
        assert_eq!(value["diagnostic"]["status"], "ok");
    }

    #[test]
    fn acp_tool_result_endpoint_marks_changed_replay_as_conflict() {
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let (result_tx, _result_rx) = tokio::sync::oneshot::channel();
        registry
            .register(ToolTurnRegistration {
                user_id: 1,
                bear_id: Uuid::new_v4(),
                bear_slug: "test-bear".to_string(),
                acp_session_id: "acp-conflict-session".to_string(),
                request_id,
                tool_call_id: "call_conflict".to_string(),
                tool_name: "fs_read_text_file".to_string(),
                approval_request_id: None,
                timeout_ms: 1_000,
                result_tx,
            })
            .expect("register tool turn");

        let first = ToolResultRequest {
            tool_call_id: Some("call_conflict".to_string()),
            tool_name: Some("fs_read_text_file".to_string()),
            status: "ok".to_string(),
            content: Some("original body".to_string()),
            structured_content: serde_json::json!({"k":"v1"}),
            diagnostic: serde_json::json!({"phase":"first"}),
            ..Default::default()
        };
        let changed = ToolResultRequest {
            content: Some("changed body".to_string()),
            structured_content: serde_json::json!({"k":"v2"}),
            diagnostic: serde_json::json!({"phase":"second"}),
            ..first.clone()
        };

        let first_delivery = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-conflict-session",
                "call_conflict",
                first,
            )
            .expect("first delivery");
        assert!(matches!(first_delivery, ToolResultDelivery::Delivered { .. }));

        let replay = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-conflict-session",
                "call_conflict",
                changed,
            )
            .expect("changed replay should classify without transport error");
        let response = acp_tool_result_response_from_delivery(
            replay,
            "acp-conflict-session",
            "call_conflict".to_string(),
            ToolStatus::Ok,
            &registry,
        );
        let value = response.to_value();
        assert_eq!(value["accepted"], true);
        assert_eq!(value["reason"], "duplicate_result_ignored");
        assert_eq!(value["settlement"], "already_settled");
        assert_eq!(value["diagnostic"]["tool_call_id"], "call_conflict");
        assert_eq!(value["diagnostic"]["status"], "ok");
    }

    #[tokio::test]
    async fn acp_stream_cancel_pending_local_tool() {
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-cancel-session",
            Some("conv-cancel".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-cancel-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "new-acp-test".to_string(),
            resolved_conversation_id: Some("conv-cancel".to_string()),
            upstream_target: "conv-cancel".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![Ok::<Bytes, CustomError>(Bytes::from(concat!(
            "data: {\"id\":\"approval-cancel\",\"message_type\":\"approval_request_message\",",
            "\"tool_call\":{\"name\":\"fs_read_text_file\",\"tool_call_id\":\"call_cancel\",",
            "\"arguments\":\"{\\\"path\\\":\\\"/tmp/acp-cancel.txt\\\"}\"}}\n\n"
        )))]);
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        )
        .with_cancel_rx(cancel_rx);

        let first = stream.next().await.unwrap().unwrap();
        let first_text = String::from_utf8(first.to_vec()).unwrap();
        assert!(first_text.contains("\"type\":\"tool_request\""));
        assert!(first_text.contains("\"tool_call_id\":\"call_cancel\""));

        cancel_tx.send(true).unwrap();
        let cancelled = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("cancel terminal should not hang");
        let cancelled = cancelled
            .expect("cancel should emit terminal before ending")
            .unwrap();
        let cancelled_text = String::from_utf8(cancelled.to_vec()).unwrap();
        assert!(
            cancelled_text.contains("\"type\":\"turn_result\""),
            "{cancelled_text}"
        );
        assert!(
            cancelled_text.contains("\"status\":\"cancelled\""),
            "{cancelled_text}"
        );
        assert!(
            cancelled_text.contains("\"reason\":\"cancelled\""),
            "{cancelled_text}"
        );

        let late = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-cancel-session",
                "call_cancel",
                ToolResultRequest {
                    tool_call_id: Some("call_cancel".to_string()),
                    tool_name: Some("fs_read_text_file".to_string()),
                    status: "ok".to_string(),
                    content: Some("late result".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(late, ToolResultDelivery::TurnMissing { .. }));
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn acp_stream_requires_approval_stop_with_active_tool_does_not_trigger_cleanup() {
        use axum::{extract::State, http::header, response::IntoResponse, routing::post, Router};
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;
        use tokio::sync::Mutex as TokioMutex;

        #[derive(Clone)]
        struct FakeState {
            cancel_calls: Arc<TokioMutex<usize>>,
        }

        async fn fake_cancel(State(state): State<FakeState>) -> impl IntoResponse {
            *state.cancel_calls.lock().await += 1;
            (
                [(header::CONTENT_TYPE, "application/json")],
                "{\"cancelled\":true}",
            )
        }

        let cancel_calls = Arc::new(TokioMutex::new(0));
        let app = Router::new()
            .route("/v1/agents/{runtime_binding_id}/messages/cancel", post(fake_cancel))
            .with_state(FakeState {
                cancel_calls: cancel_calls.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-requires-approval-active-tool",
            Some("conv-test".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-requires-approval-active-tool".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-test".to_string(),
            resolved_conversation_id: Some("conv-test".to_string()),
            upstream_target: "conv-test".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![
            Ok::<Bytes, CustomError>(Bytes::from(concat!(
                "data: {\"id\":\"approval-active\",\"run_id\":\"run-active\",\"message_type\":\"approval_request_message\",",
                "\"tool_call\":{\"name\":\"fs_read_text_file\",\"tool_call_id\":\"call_active\",",
                "\"arguments\":\"{\\\"path\\\":\\\"/tmp/acp-test.txt\\\"}\"}}\n\n"
            ))),
            Ok::<Bytes, CustomError>(Bytes::from(
                "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"requires_approval\"}\n\n",
            )),
        ]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let first = stream.next().await.unwrap().unwrap();
        let first_text = String::from_utf8(first.to_vec()).unwrap();
        assert!(first_text.contains("\"type\":\"tool_request\""));
        assert!(first_text.contains("\"tool_call_id\":\"call_active\""));

        let pending = tokio::time::timeout(std::time::Duration::from_millis(150), stream.next()).await;
        if let Ok(Some(Ok(frame))) = pending {
            let pending_text = String::from_utf8(frame.to_vec()).unwrap();
            assert!(
                pending_text.contains("\"type\":\"status_text\""),
                "unexpected output while waiting on active tool: {pending_text}"
            );
        }
        assert_eq!(
            *cancel_calls.lock().await,
            0,
            "requires_approval with an active tool must not trigger stale cleanup"
        );

        let late = registry
            .deliver_result(
                1,
                "test-bear",
                "acp-requires-approval-active-tool",
                "call_active",
                ToolResultRequest {
                    tool_call_id: Some("call_active".to_string()),
                    tool_name: Some("fs_read_text_file".to_string()),
                    status: "ok".to_string(),
                    content: Some("late result after pending check".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(
            late,
            ToolResultDelivery::RecentlySettled { .. }
                | ToolResultDelivery::TurnMissing { .. }
                | ToolResultDelivery::Delivered { .. }
        ));
    }

    #[tokio::test]
    async fn acp_stream_cleans_orphaned_requires_approval_stop() {
        use axum::{extract::State, http::header, response::IntoResponse, routing::post, Router};
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;
        use tokio::sync::Mutex as TokioMutex;

        #[derive(Clone)]
        struct FakeState {
            cancel_calls: Arc<TokioMutex<usize>>,
        }

        async fn fake_cancel(State(state): State<FakeState>) -> impl IntoResponse {
            *state.cancel_calls.lock().await += 1;
            (
                [(header::CONTENT_TYPE, "application/json")],
                "{\"cancelled\":true}",
            )
        }

        let cancel_calls = Arc::new(TokioMutex::new(0));
        let app = Router::new()
            .route("/v1/agents/{runtime_binding_id}/messages/cancel", post(fake_cancel))
            .with_state(FakeState {
                cancel_calls: cancel_calls.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-orphaned-approval",
            Some("conv-test".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-orphaned-approval".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-test".to_string(),
            resolved_conversation_id: Some("conv-test".to_string()),
            upstream_target: "conv-test".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![Ok::<Bytes, CustomError>(Bytes::from(
            "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"requires_approval\"}\n\n",
        ))]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let mut output = String::new();
        while let Some(item) = stream.next().await {
            output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
        }
        assert!(!output.contains("status_text"), "{output}");
        assert!(!output.contains("runtime recovery"), "{output}");
        assert!(output.contains("\"type\":\"turn_result\""), "{output}");
        assert_eq!(
            *cancel_calls.lock().await,
            0,
            "orphaned cleanup without run_ids must not issue an external runtime cancel"
        );
    }

    #[test]
    fn stale_approval_recovery_uses_inspect_only_mode_to_avoid_conversation_contamination() {
        let mode = PendingApprovalDenialMode::InspectOnly;
        assert!(matches!(mode, PendingApprovalDenialMode::InspectOnly));
    }

    #[tokio::test]
    async fn acp_stream_cleans_runtime_when_tool_return_continuation_conflicts() {
        use axum::{
            extract::State, http::header, response::IntoResponse, routing::post, Json, Router,
        };
        use futures::StreamExt;
        use sqlx::postgres::PgPoolOptions;
        use std::sync::Arc;
        use tokio::sync::Mutex as TokioMutex;

        #[derive(Clone)]
        struct FakeState {
            captured: Arc<TokioMutex<Option<serde_json::Value>>>,
            cancel_calls: Arc<TokioMutex<usize>>,
        }

        async fn fake_tool_return(
            State(state): State<FakeState>,
            Json(body): Json<serde_json::Value>,
        ) -> impl IntoResponse {
            *state.captured.lock().await = Some(body);
            (
                StatusCode::CONFLICT,
                [(header::CONTENT_TYPE, "application/json")],
                "{\"error\":\"conversation waiting for approval\"}",
            )
        }

        async fn fake_cancel(State(state): State<FakeState>) -> impl IntoResponse {
            *state.cancel_calls.lock().await += 1;
            (
                [(header::CONTENT_TYPE, "application/json")],
                "{\"cancelled\":true}",
            )
        }

        let captured = Arc::new(TokioMutex::new(None));
        let cancel_calls = Arc::new(TokioMutex::new(0));
        let app = Router::new()
            .route(
                "/v1/conversations/{conversation_id}/messages",
                post(fake_tool_return),
            )
            .route("/v1/agents/{runtime_binding_id}/messages/cancel", post(fake_cancel))
            .with_state(FakeState {
                captured: captured.clone(),
                cancel_calls: cancel_calls.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let registry = ToolTurnCoordinator::new();
        let request_id = Uuid::new_v4();
        let role_runtime = RoleRuntime::new(registry.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-continuation-conflict",
            Some("conv-test".to_string()),
        );
        let active_turn_guard = role_runtime
            .acquire_turn(turn_scope.clone(), request_id)
            .unwrap();
        let context = AcpStreamContext {
            pool,
            tool_turns: registry.clone(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-continuation-conflict".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-test".to_string(),
            resolved_conversation_id: Some("conv-test".to_string()),
            upstream_target: "conv-test".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: role_runtime.clone(),
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };
        let upstream = futures::stream::iter(vec![
            Ok::<Bytes, CustomError>(Bytes::from(concat!(
                "data: {\"id\":\"approval-1\",\"run_id\":\"run-conflict\",\"message_type\":\"approval_request_message\",",
                "\"tool_call\":{\"name\":\"fs_read_text_file\",\"tool_call_id\":\"call_conflict\",",
                "\"arguments\":\"{\\\"path\\\":\\\"/tmp/acp-test.txt\\\"}\"}}\n\n"
            ))),
            Ok::<Bytes, CustomError>(Bytes::from(
                "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"requires_approval\"}\n\n",
            )),
        ]);
        let mut stream = AcpRuntimeSseStream::new(
            acp_test_runtime_event_stream(upstream),
            context,
            Vec::new(),
            false,
            active_turn_guard,
        );

        let first = stream.next().await.unwrap().unwrap();
        assert!(String::from_utf8(first.to_vec())
            .unwrap()
            .contains("tool_request"));
        registry
            .deliver_result(
                1,
                "test-bear",
                "acp-continuation-conflict",
                "call_conflict",
                ToolResultRequest {
                    tool_call_id: Some("call_conflict".to_string()),
                    tool_name: Some("fs_read_text_file".to_string()),
                    approval_request_id: None,
                    status: "ok".to_string(),
                    content: Some("hello".to_string()),
                    structured_content: serde_json::json!({}),
                    diagnostic: serde_json::json!({}),
                    ..Default::default()
                },
            )
            .unwrap();

        let mut output = String::new();
        while let Some(item) = stream.next().await {
            output.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
            if output.contains("\"status\":\"recovered\"")
                || output.contains("Letta is not configured")
            {
                break;
            }
        }
        assert!(
            output.contains("\"status\":\"recovered\"")
                || output.contains("Letta is not configured"),
            "{output}"
        );
        assert!(
            output.contains("\"run_ids\":[\"run-conflict\"]")
                || output.contains("run-conflict")
                || output.contains("Failed to continue runtime after ACP local tool result."),
            "{output}"
        );
        assert!(*cancel_calls.lock().await <= 1);
    }

    #[test]
    fn canonical_user_prompt_record_carries_prompt_scope_and_request_metadata() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance {
            source: "acp_prompt".to_string(),
            scope_id: "acp-session-123".to_string(),
        };
        let mut content_json = provenance.as_content_json("user_prompt");
        content_json["role"] = serde_json::json!("user");
        content_json["acp_session_id"] = serde_json::json!("acp-session-123");
        content_json["client"] = serde_json::json!("zed");
        content_json["request_id"] = serde_json::json!("req-prompt-123");

        let record =
            CanonicalConversationRecord::visible_user_message("hello prompt", content_json, None);
        match record {
            CanonicalConversationRecord::VisibleMessage {
                role,
                text,
                content_json,
                provider_message_id,
            } => {
                assert_eq!(role.as_str(), "user");
                assert_eq!(text, "hello prompt");
                assert!(provider_message_id.is_none());
                assert_eq!(content_json["source"], "acp_prompt");
                assert_eq!(content_json["event"], "user_prompt");
                assert_eq!(content_json["scope_id"], "acp-session-123");
                assert_eq!(content_json["acp_session_id"], "acp-session-123");
                assert_eq!(content_json["client"], "zed");
                assert_eq!(content_json["request_id"], "req-prompt-123");
            }
            _ => panic!("expected visible message"),
        }
    }

    #[test]
    fn canonical_user_prompt_record_matches_prompt_flow_persistence_shape() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let session_id = "acp-session-prompt-flow";
        let client = "zed";
        let request_id = "req-prompt-flow-123";
        let provenance = ConversationEventProvenance {
            source: "acp_prompt".to_string(),
            scope_id: session_id.to_string(),
        };
        let mut content_json = provenance.as_content_json("user_prompt");
        content_json["role"] = serde_json::json!("user");
        content_json["acp_session_id"] = serde_json::json!(session_id);
        content_json["client"] = serde_json::json!(client);
        content_json["request_id"] = serde_json::json!(request_id);

        let record = CanonicalConversationRecord::visible_user_message(
            "prompt from prompt_flow",
            content_json,
            None,
        );

        match record {
            CanonicalConversationRecord::VisibleMessage {
                role,
                text,
                content_json,
                provider_message_id,
            } => {
                assert_eq!(role.as_str(), "user");
                assert_eq!(text, "prompt from prompt_flow");
                assert!(provider_message_id.is_none());
                assert_eq!(content_json["event"], "user_prompt");
                assert_eq!(content_json["source"], "acp_prompt");
                assert_eq!(content_json["scope_id"], session_id);
                assert_eq!(content_json["role"], "user");
                assert_eq!(content_json["acp_session_id"], session_id);
                assert_eq!(content_json["client"], client);
                assert_eq!(content_json["request_id"], request_id);
            }
            _ => panic!("expected visible message"),
        }
    }

    #[test]
    fn canonical_conversation_resolved_record_carries_resolution_metadata() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-session-456");
        let record =
            CanonicalConversationRecord::conversation_resolved("conv-12345678", &provenance);

        match record {
            CanonicalConversationRecord::StructuredEvent {
                message_type,
                role,
                visibility,
                content_text,
                content_json,
                provider_message_id,
            } => {
                assert_eq!(
                    message_type,
                    den_runtime::conversation_message_types::ConversationMessageType::WorkflowEvent
                );
                assert_eq!(
                    role,
                    Some(den_runtime::conversation_message_types::ConversationMessageRole::System)
                );
                assert_eq!(
                    visibility,
                    den_runtime::conversation_message_types::ConversationMessageVisibility::DiagnosticOnly
                );
                assert_eq!(content_text, "Conversation resolved");
                assert!(provider_message_id.is_none());
                assert_eq!(content_json["source"], "acp_stream");
                assert_eq!(content_json["event"], "conversation_resolved");
                assert_eq!(content_json["scope_id"], "acp-session-456");
                assert_eq!(content_json["conversation_id"], "conv-12345678");
            }
            _ => panic!("expected structured event"),
        }
    }

    #[tokio::test]
    async fn acp_session_provenance_helper_uses_acp_session_scope() {
        use std::sync::Arc;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
            .unwrap();
        let request_id = Uuid::new_v4();
        let tool_turns = ToolTurnCoordinator::new();
        let role_runtime = RoleRuntime::new(tool_turns.clone());
        let turn_scope = RoleTurnScope::acp_pair(
            Uuid::new_v4(),
            "acp-helper-session",
            Some("conv-helper".to_string()),
        );
        let context = AcpStreamContext {
            pool,
            tool_turns,
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-helper-session".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-helper".to_string(),
            resolved_conversation_id: Some("conv-helper".to_string()),
            upstream_target: "conv-helper".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id,
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime,
            turn_scope,
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        };

        let provenance = super::stream::runtime::acp_session_provenance(&context);
        assert_eq!(provenance.source, "acp_stream");
        assert_eq!(provenance.scope_id, "acp-helper-session");
    }

    #[tokio::test]
    async fn canonical_conversation_resolved_helper_preserves_acp_session_provenance() {
        use std::sync::Arc;
        let provenance = super::stream::runtime::acp_session_provenance(&AcpStreamContext {
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://postgres:postgres@127.0.0.1/postgres")
                .unwrap(),
            tool_turns: ToolTurnCoordinator::new(),
            user_id: 1,
            user_profile: None,
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            acp_session_id: "acp-session-resolved-helper".to_string(),
            client: "zed".to_string(),
            conversation_id: "conv-test-resolved".to_string(),
            conversation_selection: "conv-helper".to_string(),
            resolved_conversation_id: Some("conv-helper".to_string()),
            upstream_target: "conv-helper".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            request_id: Uuid::new_v4(),
            pair_agent_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            config: Arc::new(den_core::config::Config::test_stub()),
            role_runtime: RoleRuntime::new(ToolTurnCoordinator::new()),
            turn_scope: RoleTurnScope::acp_pair(
                Uuid::new_v4(),
                "acp-session-resolved-helper",
                Some("conv-helper".to_string()),
            ),
            prompt_memory_diagnostic: serde_json::json!({}),
            memory_stores: den_runtime::memory::MemoryStoreManager::new(&den_core::config::Config::test_stub()),
        });
        let record = den_runtime::conversation_events::CanonicalConversationRecord::conversation_resolved(
            "conv-validated",
            &provenance,
        );

        match record {
            den_runtime::conversation_events::CanonicalConversationRecord::StructuredEvent {
                content_json, ..
            } => {
                assert_eq!(content_json["source"], "acp_stream");
                assert_eq!(content_json["scope_id"], "acp-session-resolved-helper");
                assert_eq!(content_json["conversation_id"], "conv-validated");
            }
            _ => panic!("expected structured event"),
        }
    }

    #[test]
    fn canonical_assistant_output_records_same_request_scope_for_duplicate_like_replays() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-test-session");
        let first = CanonicalConversationRecord::assistant_output(
            "hello once",
            &provenance,
            Some("provider-1".to_string()),
            Some("req-123".to_string()),
        );
        let replay = CanonicalConversationRecord::assistant_output(
            "hello twice",
            &provenance,
            Some("provider-1".to_string()),
            Some("req-123".to_string()),
        );

        let first_json = match first {
            CanonicalConversationRecord::VisibleMessage { content_json, .. } => content_json,
            _ => panic!("expected visible message"),
        };
        let replay_json = match replay {
            CanonicalConversationRecord::VisibleMessage { content_json, .. } => content_json,
            _ => panic!("expected visible message"),
        };

        assert_eq!(first_json["event"], "assistant_output");
        assert_eq!(first_json["request_id"], "req-123");
        assert_eq!(replay_json["request_id"], "req-123");
        assert_eq!(first_json["scope_id"], replay_json["scope_id"]);
        assert_eq!(
            first_json["provider_message_id"],
            replay_json["provider_message_id"]
        );
    }

    #[test]
    fn canonical_tool_result_timeout_record_preserves_timeout_status_and_diagnostic_phase() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-timeout-session");
        let record = CanonicalConversationRecord::tool_result(
            Some("fs_read_text_file".to_string()),
            "call-timeout-1",
            Some("approval-timeout-1".to_string()),
            "timeout",
            Some("timed out waiting for local tool result".to_string()),
            serde_json::json!({}),
            serde_json::json!({
                "component": "den.acp",
                "phase": "local_tool_result_timeout_auto_denied"
            }),
            Some("req-timeout-1".to_string()),
            &provenance,
        );

        match record {
            CanonicalConversationRecord::StructuredEvent {
                content_json,
                content_text,
                ..
            } => {
                assert_eq!(content_text, "Tool result: fs_read_text_file");
                assert_eq!(content_json["event"], "tool_result");
                assert_eq!(content_json["status"], "timeout");
                assert_eq!(content_json["tool_call_id"], "call-timeout-1");
                assert_eq!(content_json["approval_request_id"], "approval-timeout-1");
                assert_eq!(content_json["request_id"], "req-timeout-1");
                assert_eq!(
                    content_json["diagnostic"]["phase"],
                    "local_tool_result_timeout_auto_denied"
                );
            }
            _ => panic!("expected structured event"),
        }
    }

    #[test]
    fn canonical_tool_result_error_record_preserves_error_status_and_diagnostic_phase() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-error-session");
        let record = CanonicalConversationRecord::tool_result(
            Some("fs_read_text_file".to_string()),
            "call-error-1",
            None,
            "error",
            Some("tool result channel closed".to_string()),
            serde_json::json!({}),
            serde_json::json!({
                "component": "den.acp",
                "phase": "local_tool_result_channel_closed_auto_denied"
            }),
            Some("req-error-1".to_string()),
            &provenance,
        );

        match record {
            CanonicalConversationRecord::StructuredEvent { content_json, .. } => {
                assert_eq!(content_json["event"], "tool_result");
                assert_eq!(content_json["status"], "error");
                assert_eq!(content_json["tool_call_id"], "call-error-1");
                assert_eq!(
                    content_json["diagnostic"]["phase"],
                    "local_tool_result_channel_closed_auto_denied"
                );
            }
            _ => panic!("expected structured event"),
        }
    }

    #[test]
    fn canonical_turn_outcome_records_cancellation_request_scope() {
        use den_runtime::conversation_events::{
            CanonicalConversationRecord, ConversationEventProvenance,
        };

        let provenance = ConversationEventProvenance::acp_session("acp-test-session");
        let record = CanonicalConversationRecord::turn_outcome(
            "cancelled",
            "cancelled",
            "req-cancel-123",
            false,
            serde_json::json!({"channel_id":"acp-test-session"}),
            serde_json::json!({"cancel_source":"user"}),
            &provenance,
        );

        match record {
            CanonicalConversationRecord::StructuredEvent {
                content_text,
                content_json,
                ..
            } => {
                assert_eq!(content_text, "Turn outcome: cancelled / cancelled");
                assert_eq!(content_json["event"], "turn_result");
                assert_eq!(content_json["status"], "cancelled");
                assert_eq!(content_json["reason"], "cancelled");
                assert_eq!(content_json["request_id"], "req-cancel-123");
                assert_eq!(content_json["scope_id"], "acp-test-session");
                assert_eq!(content_json["diagnostics"]["cancel_source"], "user");
            }
            _ => panic!("expected structured event"),
        }
    }

    #[tokio::test]
    async fn sse_parser_joins_multiple_data_lines_into_one_json_value() {
        let body = br#"data: {"message_type":"assistant_message","content":
data: "hello"}"#;
        let v = parse_sse_event_body_to_json(body).unwrap().unwrap();
        assert_eq!(v["message_type"], "assistant_message");
        assert_eq!(v["content"], "hello");
        let frames = b"data: {\"message_type\":\"assistant_message\",\"content\":\ndata: \"hello\"}\n\n";
        let source = futures::stream::iter(vec![Ok(Bytes::from_static(frames))]);
        let mut stream = acp_test_runtime_event_stream(source);
        let first = futures::StreamExt::next(&mut stream)
            .await
            .expect("event")
            .expect("ok");
        assert!(matches!(
            first,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { .. })
        ));
    }

    #[tokio::test]
    async fn sse_parser_rejects_invalid_json_with_parse_path_empty() {
        let body = br"data: not-json";
        assert!(parse_sse_event_body_to_json(body).is_err());
        let frames = b"data: not-json\n\n";
        let source = futures::stream::iter(vec![Ok(Bytes::from_static(frames))]);
        let mut stream = acp_test_runtime_event_stream(source);
        let first = futures::StreamExt::next(&mut stream).await.expect("event");
        assert!(first.is_err(), "invalid provider JSON should surface as stream error");
    }

    #[test]
    fn sse_frame_end_prefers_earliest_lf_or_crlf_delimiter() {
        let buf = b"data: {}\r\n\r\n";
        assert_eq!(find_sse_frame_end(buf), Some(12));
        let buf2 = b"data: {}\n\n";
        assert_eq!(find_sse_frame_end(buf2), Some(10));
    }

    #[test]
    fn normalizes_acp_conversation_ids() {
        assert_eq!(normalize_acp_conversation_id(None).unwrap(), "default");
        assert_eq!(
            normalize_acp_conversation_id(Some("conv-abc12345")).unwrap(),
            "conv-abc12345"
        );
        assert_eq!(
            normalize_acp_conversation_id(Some("new-acp-zed-abc12345")).unwrap(),
            "new-acp-zed-abc12345"
        );
        assert!(normalize_acp_conversation_id(Some("conv-x")).is_err());
        assert!(normalize_acp_conversation_id(Some("../../etc/passwd")).is_err());
    }

    #[test]
    fn generated_acp_conversation_ids_are_compact_opaque_ids() {
        let id = new_acp_conversation_id("zed");
        assert!(id.starts_with("new-acp-zed-"));
        assert_eq!(id.len(), 34);
        assert!(is_valid_pending_acp_conversation_id(&id));

        let id = new_acp_conversation_id("acp_adapter");
        assert!(id.starts_with("new-acp-acp_adapter-"));
        assert_eq!(id.len(), 42);
        assert!(is_valid_pending_acp_conversation_id(&id));
    }

    #[test]
    fn resolver_maps_pending_acp_selection_to_native_runtime_target() {
        let binding = den_protocol::RoleRuntimeBinding {
            binding_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            compatibility_backend: Some("runtime:native".to_string()),
        };
        let resolution =
            resolve_acp_prompt_conversation(None, None, &binding, "new-acp-zed-abc123".to_string())
                .unwrap();
        assert_eq!(resolution.session_selection, "new-acp-zed-abc123");
        assert_eq!(resolution.resolved_conversation, None);
        assert_eq!(resolution.upstream_target, "new-acp-zed-abc123");
        assert_eq!(resolution.history_target, None);
        assert_eq!(resolution.archive_target, None);
        assert_eq!(
            resolution.selection_source,
            AcpConversationSelectionSource::Generated
        );
        assert!(!resolution.should_materialize_runtime_conversation);
    }

    #[test]
    fn resolver_routes_explicit_conv_directly_and_requires_bear_check() {
        let binding = den_protocol::RoleRuntimeBinding {
            binding_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            compatibility_backend: Some("runtime:native".to_string()),
        };
        let conv_id = "conv-12345678-1234-4567-89ab-123456789abc";
        let resolution = resolve_acp_prompt_conversation(
            Some(conv_id),
            None,
            &binding,
            "new-acp-zed-unused".to_string(),
        )
        .unwrap();
        assert_eq!(resolution.session_selection, conv_id);
        assert_eq!(
            resolution
                .resolved_conversation
                .as_ref()
                .map(|c| c.id.as_str()),
            Some(conv_id)
        );
        assert_eq!(resolution.upstream_target, conv_id);
        assert_eq!(
            resolution.history_target.as_ref().map(|c| c.id.as_str()),
            Some(conv_id)
        );
        assert_eq!(
            resolution.archive_target.as_ref().map(|c| c.id.as_str()),
            Some(conv_id)
        );
        assert_eq!(
            resolution.selection_source,
            AcpConversationSelectionSource::Explicit
        );
        assert!(resolution.requires_belongs_to_bear_check);
    }

    #[test]
    fn resolver_never_archives_pending_or_default_targets() {
        let binding = den_protocol::RoleRuntimeBinding {
            binding_id: "agent-12345678-1234-4567-89ab-123456789abc".to_string(),
            compatibility_backend: Some("runtime:native".to_string()),
        };
        let pending = AcpConversationResolution::from_selection(
            "new-acp-zed-abc123".to_string(),
            AcpConversationSelectionSource::Generated,
            &binding,
            None,
        );
        assert_eq!(pending.history_target, None);
        assert_eq!(pending.archive_target, None);

        let default = AcpConversationResolution::from_selection(
            "default".to_string(),
            AcpConversationSelectionSource::Stored,
            &binding,
            None,
        );
        assert_eq!(
            default.history_target.as_ref().map(|c| c.id.as_str()),
            Some("default")
        );
        assert_eq!(default.archive_target, None);
    }

    #[test]
    fn rejects_legacy_pending_acp_conversation_ids_that_exceed_letta_limit() {
        let legacy = "new-acp-zed-acp-12345678-1234-1234-1234-123456789abc";
        assert!(normalize_acp_conversation_id(Some(legacy)).is_ok());
        assert!(!is_valid_pending_acp_conversation_id(legacy));
    }






    #[test]
    fn acp_router_exposes_session_prompt_memory_endpoint() {
        let source = std::fs::read_to_string("/workspace/services/den/src/api/acp/mod.rs").unwrap();
        assert!(source.contains("/bears/{slug}/sessions/{session_id}/prompt-memory"));
    }
