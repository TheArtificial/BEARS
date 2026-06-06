use super::den_tools::*;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashSet;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        bears::BearAgentRole,
        prompt_memory_block_store::{upsert_prompt_memory_block, PromptMemoryBlockWrite},
        prompt_memory_blocks::{
            PromptMemoryBlockScope, PromptMemoryBlockState, PromptMemoryBlockType,
        },
        den_tools::DenToolChannelContext,
        tools::{
            memory_read::memory_status_value,
            payloads::bear_environment_payload,
            prompt_memory::prompt_memory_list,
            prompt_memory::prompt_memory_patch,
            prompt_memory::prompt_memory_upsert,
            session::{authorize_tool_for_role, DenToolInvocationContext},
            web::web_search_inner,
        },
    },
};
use serde_json::json;

fn names_for_role(role: BearAgentRole) -> HashSet<&'static str> {
    builtin_den_tool_descriptors_for_role(role)
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect()
}

#[tokio::test]
async fn prompt_memory_tools_round_trip_through_store() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
    let pool = match PgPoolOptions::new().connect(&database_url).await {
        Ok(pool) => pool,
        Err(_) => return,
    };
    let migrate = sqlx::migrate!("./migrations").run(&pool).await;
    if migrate.is_err() {
        return;
    }
    let bear_id = Uuid::new_v4();
    let context = DenToolInvocationContext {
        bear_id,
        bear_slug: "test-bear".to_string(),
        role_agent_id: "agent-test".to_string(),
        agent_role: Some(BearAgentRole::Pair),
        user_id: 1,
        username: Some("tester".to_string()),
        membership_role: Some("owner".to_string()),
        conversation_id: "conv-test".to_string(),
        session_id: "sess-test".to_string(),
        acp_session_id: Some("sess-test".to_string()),
        conversation_selection: None,
        runtime_target: None,
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: None,
        activity: None,
        runtime: None,
        context_budget: None,
        request_id: Some("req-test".to_string()),
        channel: DenToolChannelContext {
            family: Some("acp".to_string()),
            client: Some("zed".to_string()),
            protocol: Some("acp".to_string()),
        },
    };
    let upsert = prompt_memory_upsert(
        &pool,
        &context,
        BearAgentRole::Pair,
        json!({
            "block_id": format!("pm-{}", Uuid::new_v4()),
            "scope": "session",
            "block_type": "session_focus",
            "session_id": "sess-test",
            "title": "Current focus",
            "body": "Prioritize persisted prompt memory runtime wiring.",
            "priority": 7
        }),
    )
    .await
    .expect("upsert prompt memory block");
    assert_eq!(upsert["status"], "ok");
    let block_id = upsert["block_id"].as_str().unwrap().to_string();
    let listed = prompt_memory_list(&pool, &context, BearAgentRole::Pair, json!({}))
        .await
        .expect("list prompt memory blocks");
    assert!(listed["blocks"].as_array().unwrap().iter().any(|b| b["id"] == block_id));
    let patched = prompt_memory_patch(
        &pool,
        &context,
        BearAgentRole::Pair,
        json!({
            "block_id": block_id,
            "state": "archived",
            "title": "Current focus (archived)",
            "body": "Archived prompt memory block.",
            "priority": 1
        }),
    )
    .await
    .expect("patch prompt memory block");
    assert_eq!(patched["state"], "archived");
    let listed_active = prompt_memory_list(&pool, &context, BearAgentRole::Pair, json!({}))
        .await
        .expect("list active prompt memory blocks");
    assert!(!listed_active["blocks"].as_array().unwrap().iter().any(|b| b["id"] == patched["block_id"]));
    let listed_all = prompt_memory_list(
        &pool,
        &context,
        BearAgentRole::Pair,
        json!({"include_archived": true}),
    )
    .await
    .expect("list all prompt memory blocks");
    assert!(listed_all["blocks"].as_array().unwrap().iter().any(|b| b["id"] == patched["block_id"]));
}

#[tokio::test]
async fn prompt_memory_runtime_selection_prefers_session_then_surface_then_role_then_bear() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
    let pool = match PgPoolOptions::new().connect(&database_url).await {
        Ok(pool) => pool,
        Err(_) => return,
    };
    if sqlx::migrate!("./migrations").run(&pool).await.is_err() {
        return;
    }
    let bear_id = Uuid::new_v4();
    let role_slug = BearAgentRole::Pair.as_str();
    let session_id = format!("sess-{}", Uuid::new_v4());
    let work_surface = format!("ws-{}", Uuid::new_v4());
    let ids = [
        format!("pm-bear-{}", Uuid::new_v4()),
        format!("pm-role-{}", Uuid::new_v4()),
        format!("pm-surface-{}", Uuid::new_v4()),
        format!("pm-session-{}", Uuid::new_v4()),
    ];
    let writes = vec![
        PromptMemoryBlockWrite {
            block_id: ids[0].clone(),
            bear_id: Some(bear_id),
            role_slug: Some(role_slug.to_string()),
            scope: PromptMemoryBlockScope::BearWide,
            block_type: PromptMemoryBlockType::RoleGuidance,
            state: PromptMemoryBlockState::Active,
            work_surface: None,
            session_id: None,
            title: "Bear".to_string(),
            body: "Bear-wide guidance".to_string(),
            priority: 1,
            created_by_user_id: Some(1),
            supersedes_block_id: None,
            metadata: json!({}),
        },
        PromptMemoryBlockWrite {
            block_id: ids[1].clone(),
            bear_id: Some(bear_id),
            role_slug: Some(role_slug.to_string()),
            scope: PromptMemoryBlockScope::RoleLocal,
            block_type: PromptMemoryBlockType::RoleGuidance,
            state: PromptMemoryBlockState::Active,
            work_surface: None,
            session_id: None,
            title: "Role".to_string(),
            body: "Role guidance".to_string(),
            priority: 1,
            created_by_user_id: Some(1),
            supersedes_block_id: None,
            metadata: json!({}),
        },
        PromptMemoryBlockWrite {
            block_id: ids[2].clone(),
            bear_id: Some(bear_id),
            role_slug: Some(role_slug.to_string()),
            scope: PromptMemoryBlockScope::WorkSurface,
            block_type: PromptMemoryBlockType::WorkSurfaceContext,
            state: PromptMemoryBlockState::Active,
            work_surface: Some(work_surface.clone()),
            session_id: None,
            title: "Surface".to_string(),
            body: "Work-surface context".to_string(),
            priority: 1,
            created_by_user_id: Some(1),
            supersedes_block_id: None,
            metadata: json!({}),
        },
        PromptMemoryBlockWrite {
            block_id: ids[3].clone(),
            bear_id: Some(bear_id),
            role_slug: Some(role_slug.to_string()),
            scope: PromptMemoryBlockScope::Session,
            block_type: PromptMemoryBlockType::SessionFocus,
            state: PromptMemoryBlockState::Active,
            work_surface: None,
            session_id: Some(session_id.clone()),
            title: "Session".to_string(),
            body: "Session focus".to_string(),
            priority: 1,
            created_by_user_id: Some(1),
            supersedes_block_id: None,
            metadata: json!({}),
        },
    ];
    for write in &writes {
        upsert_prompt_memory_block(&pool, write).await.expect("seed prompt memory block");
    }
    let selection = crate::core::prompt_memory_block_store::select_prompt_memory_blocks_for_runtime(
        &pool,
        crate::core::prompt_memory_block_store::PromptMemoryBlockQuery {
            bear_id: Some(bear_id),
            role_slug,
            session_id: &session_id,
            work_surfaces: std::slice::from_ref(&work_surface),
        },
    )
    .await
    .expect("runtime selection");
    let compiled = crate::core::prompt_memory_blocks::compile_prompt_memory_blocks(
        &selection.blocks,
        crate::core::prompt_memory_blocks::PromptMemoryCompilationInput {
            role: role_slug,
            work_surfaces: std::slice::from_ref(&work_surface),
            session_id: &session_id,
            max_blocks: 4,
        },
    );
    let included_ids = compiled
        .included_blocks
        .iter()
        .map(|block| block.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(included_ids, vec![ids[3].clone(), ids[2].clone(), ids[1].clone(), ids[0].clone()]);
    assert_eq!(selection.diagnostic["matched_count"], 4);
}

#[tokio::test]
async fn prompt_memory_upsert_archives_superseded_block() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
    let pool = match PgPoolOptions::new().connect(&database_url).await {
        Ok(pool) => pool,
        Err(_) => return,
    };
    if sqlx::migrate!("./migrations").run(&pool).await.is_err() {
        return;
    }
    let bear_id = Uuid::new_v4();
    let context = DenToolInvocationContext {
        bear_id,
        bear_slug: "test-bear".to_string(),
        role_agent_id: "agent-test".to_string(),
        agent_role: Some(BearAgentRole::Pair),
        user_id: 1,
        username: Some("tester".to_string()),
        membership_role: Some("owner".to_string()),
        conversation_id: "conv-test".to_string(),
        session_id: "sess-test".to_string(),
        acp_session_id: Some("sess-test".to_string()),
        conversation_selection: None,
        runtime_target: None,
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: None,
        activity: None,
        runtime: None,
        context_budget: None,
        request_id: Some("req-test".to_string()),
        channel: DenToolChannelContext {
            family: Some("acp".to_string()),
            client: Some("zed".to_string()),
            protocol: Some("acp".to_string()),
        },
    };
    let original_block_id = format!("pm-original-{}", Uuid::new_v4());
    prompt_memory_upsert(
        &pool,
        &context,
        BearAgentRole::Pair,
        json!({
            "block_id": original_block_id,
            "scope": "session",
            "block_type": "session_focus",
            "session_id": "sess-test",
            "title": "Original focus",
            "body": "Original body",
            "priority": 3
        }),
    )
    .await
    .expect("upsert original block");
    let replacement = prompt_memory_upsert(
        &pool,
        &context,
        BearAgentRole::Pair,
        json!({
            "block_id": format!("pm-replacement-{}", Uuid::new_v4()),
            "scope": "session",
            "block_type": "session_focus",
            "session_id": "sess-test",
            "title": "Replacement focus",
            "body": "Replacement body",
            "priority": 5,
            "supersedes_block_id": original_block_id
        }),
    )
    .await
    .expect("upsert replacement block");
    assert_eq!(replacement["superseded_archived_count"], 1);
    let listed_all = prompt_memory_list(&pool, &context, BearAgentRole::Pair, json!({"include_archived": true}))
        .await
        .expect("list prompt memory blocks");
    let original = listed_all["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["id"] == replacement["supersedes_block_id"])
        .expect("original block should still be listed when archived included");
    assert_eq!(original["state"], "archived");
}

#[tokio::test]
async fn prompt_memory_upsert_archives_conflicting_active_block_in_same_scope() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
    let pool = match PgPoolOptions::new().connect(&database_url).await {
        Ok(pool) => pool,
        Err(_) => return,
    };
    if sqlx::migrate!("./migrations").run(&pool).await.is_err() {
        return;
    }
    let bear_id = Uuid::new_v4();
    let context = DenToolInvocationContext {
        bear_id,
        bear_slug: "test-bear".to_string(),
        role_agent_id: "agent-test".to_string(),
        agent_role: Some(BearAgentRole::Pair),
        user_id: 1,
        username: Some("tester".to_string()),
        membership_role: Some("owner".to_string()),
        conversation_id: "conv-test".to_string(),
        session_id: "sess-test".to_string(),
        acp_session_id: Some("sess-test".to_string()),
        conversation_selection: None,
        runtime_target: None,
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: None,
        activity: None,
        runtime: None,
        context_budget: None,
        request_id: Some("req-test".to_string()),
        channel: DenToolChannelContext {
            family: Some("acp".to_string()),
            client: Some("zed".to_string()),
            protocol: Some("acp".to_string()),
        },
    };
    prompt_memory_upsert(
        &pool,
        &context,
        BearAgentRole::Pair,
        json!({
            "block_id": format!("pm-conflict-a-{}", Uuid::new_v4()),
            "scope": "session",
            "block_type": "session_focus",
            "session_id": "sess-test",
            "title": "First focus",
            "body": "First body",
            "priority": 3
        }),
    )
    .await
    .expect("upsert first block");
    let replacement = prompt_memory_upsert(
        &pool,
        &context,
        BearAgentRole::Pair,
        json!({
            "block_id": format!("pm-conflict-b-{}", Uuid::new_v4()),
            "scope": "session",
            "block_type": "session_focus",
            "session_id": "sess-test",
            "title": "Second focus",
            "body": "Second body",
            "priority": 8
        }),
    )
    .await
    .expect("upsert second block");
    assert_eq!(replacement["conflicting_archived_count"], 1);
    let active = prompt_memory_list(
        &pool,
        &context,
        BearAgentRole::Pair,
        json!({"scope": "session", "block_type": "session_focus", "session_id": "sess-test"}),
    )
    .await
    .expect("list active session prompt memory blocks");
    assert_eq!(active["count"], 1);
}

#[tokio::test]
async fn memory_status_includes_prompt_memory_diagnostic_summary() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
    let pool = match PgPoolOptions::new().connect(&database_url).await {
        Ok(pool) => pool,
        Err(_) => return,
    };
    if sqlx::migrate!("./migrations").run(&pool).await.is_err() {
        return;
    }
    let bear_id = Uuid::new_v4();
    let context = DenToolInvocationContext {
        bear_id,
        bear_slug: "test-bear".to_string(),
        role_agent_id: "agent-test".to_string(),
        agent_role: Some(BearAgentRole::Pair),
        user_id: 1,
        username: Some("tester".to_string()),
        membership_role: Some("owner".to_string()),
        conversation_id: "conv-test".to_string(),
        session_id: "sess-test".to_string(),
        acp_session_id: Some("sess-test".to_string()),
        conversation_selection: None,
        runtime_target: None,
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: None,
        activity: None,
        runtime: None,
        context_budget: None,
        request_id: Some("req-test".to_string()),
        channel: DenToolChannelContext {
            family: Some("acp".to_string()),
            client: Some("zed".to_string()),
            protocol: Some("acp".to_string()),
        },
    };
    prompt_memory_upsert(
        &pool,
        &context,
        BearAgentRole::Pair,
        json!({
            "block_id": format!("pm-status-{}", Uuid::new_v4()),
            "scope": "session",
            "block_type": "session_focus",
            "session_id": "sess-test",
            "title": "Status focus",
            "body": "Status body",
            "priority": 4
        }),
    )
    .await
    .expect("upsert status block");
    let config = Config::test_stub();
    let status = memory_status_value(&config, &context, BearAgentRole::Pair, &pool)
        .await
        .expect("memory status value");
    assert_eq!(status["prompt_memory_diagnostic"]["source"], "prompt_memory_blocks");
    assert_eq!(status["prompt_memory_diagnostic"]["active_by_scope"]["session"], 1);
}

#[test]
fn provider_names_are_safe_and_unique() {
    let descriptors = builtin_den_tool_descriptors();
    let mut provider_names = HashSet::new();
    for descriptor in descriptors {
        assert!(
            descriptor
                .provider_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "provider name must be Letta/provider-safe: {}",
            descriptor.provider_name
        );
        assert!(!descriptor.provider_name.contains('.'));
        assert!(!descriptor.provider_name.contains('/'));
        assert!(
            provider_names.insert(descriptor.provider_name.clone()),
            "duplicate provider name: {}",
            descriptor.provider_name
        );
    }
}

#[test]
fn canonical_dotted_names_map_to_provider_safe_aliases() {
    let descriptors = builtin_den_tool_descriptors();
    let task = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_TASK_WRITE_INTENT)
        .expect("task intent descriptor exists");
    assert_eq!(task.provider_name, "den_task_write_intent");

    let skill = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_SKILL_PROPOSE)
        .expect("skill proposal descriptor exists");
    assert_eq!(skill.provider_name, "den_skill_propose");

    let conversation_title = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_CONVERSATION_SET_TITLE)
        .expect("conversation title descriptor exists");
    assert_eq!(
        conversation_title.provider_name,
        DEN_CONVERSATION_SET_TITLE_PROVIDER
    );

    let web_fetch = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_WEB_FETCH)
        .expect("web fetch descriptor exists");
    assert_eq!(web_fetch.provider_name, DEN_WEB_FETCH_PROVIDER);

    let web_search = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_WEB_SEARCH)
        .expect("web search descriptor exists");
    assert_eq!(web_search.provider_name, DEN_WEB_SEARCH_PROVIDER);

    let bear_environment = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_BEAR_ENVIRONMENT)
        .expect("bear environment descriptor exists");
    assert_eq!(
        bear_environment.provider_name,
        DEN_BEAR_ENVIRONMENT_PROVIDER
    );

    let situation = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_SITUATION_GET)
        .expect("situation descriptor exists");
    assert_eq!(situation.provider_name, DEN_SITUATION_GET_PROVIDER);
    assert_eq!(situation.provider_name, "session_info");
    assert_ne!(situation.provider_name, "situation_get");
    assert_ne!(situation.provider_name, "den_situation_get");

    let memory_browse = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_MEMORY_TREE)
        .expect("memory browse descriptor exists");
    assert_eq!(memory_browse.provider_name, DEN_MEMORY_TREE_PROVIDER);
    assert_eq!(memory_browse.provider_name, "memory_browse");
    assert_ne!(memory_browse.provider_name, "memory_tree");

    let memory = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_MEMORY_WRITE_ENTRY)
        .expect("memory write descriptor exists");
    assert_eq!(memory.provider_name, DEN_MEMORY_WRITE_ENTRY_PROVIDER);

    let update_plan = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_WORK_PLAN_UPDATE)
        .expect("work plan update descriptor exists");
    assert_eq!(update_plan.provider_name, DEN_WORK_PLAN_UPDATE_PROVIDER);
    assert_eq!(update_plan.provider_name, "update_plan");

    let enter_plan_mode = descriptors
        .iter()
        .find(|descriptor| descriptor.name == DEN_PLAN_MODE_ENTER)
        .expect("enter plan mode descriptor exists");
    assert_eq!(enter_plan_mode.provider_name, DEN_PLAN_MODE_ENTER_PROVIDER);
    assert_eq!(enter_plan_mode.provider_name, "enter_plan_mode");
}

#[test]
fn den_server_tools_advertise_semantic_aliases_not_legacy_den_prefixes() {
    let provider_names = builtin_den_tool_descriptors_for_role(BearAgentRole::Pair)
        .into_iter()
        .map(|descriptor| descriptor.provider_name)
        .collect::<HashSet<_>>();
    assert!(provider_names.contains("session_info"));
    assert!(provider_names.contains("bear_environment"));
    assert!(provider_names.contains("set_conversation_title"));
    assert!(provider_names.contains("web_search"));
    assert!(provider_names.contains("memory_browse"));
    assert!(provider_names.contains("memory_read"));
    assert!(provider_names.contains("update_plan"));
    assert!(provider_names.contains("enter_plan_mode"));
    assert!(provider_names.contains("record_plan_approval"));
    assert!(provider_names.contains("exit_plan_mode"));
    assert!(provider_names.contains("cancel_plan_mode"));
    assert!(!provider_names.contains("situation_get"));
    assert!(!provider_names.contains("memory_tree"));
    assert!(!provider_names.contains("den_situation_get"));
    assert!(!provider_names.contains("den_web_search"));
    assert!(!provider_names.contains("den_memory_read"));
    assert!(!provider_names.contains("den_work_plan_update"));
    assert!(!provider_names.contains("den_plan_mode_enter"));
}

#[test]
fn bear_environment_payload_exposes_baseline_sections() {
    let context = DenToolInvocationContext {
        bear_id: Uuid::nil(),
        bear_slug: "meta".to_string(),
        role_agent_id: "agent-123".to_string(),
        agent_role: Some(BearAgentRole::Pair),
        user_id: 7,
        username: Some("gerwitz".to_string()),
        membership_role: Some("admin".to_string()),
        conversation_id: "conv-123".to_string(),
        session_id: "sess-123".to_string(),
        acp_session_id: Some("acp-123".to_string()),
        conversation_selection: Some("conv-123".to_string()),
        runtime_target: Some("conv-123".to_string()),
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: Some(json!({ "mode_label": "Write" })),
        activity: None,
        runtime: Some(json!({
            "state": "running",
            "active_turn": { "present": true, "pending_obligations": 0 }
        })),
        context_budget: Some(json!({ "status": "unavailable" })),
        request_id: Some("req-123".to_string()),
        channel: DenToolChannelContext {
            family: Some("acp".to_string()),
            client: Some("api-direct".to_string()),
            protocol: Some("acp".to_string()),
        },
    };
    let payload = bear_environment_payload(
        &context,
        &Config::test_stub(),
        BearAgentRole::Pair,
        None,
        2,
        json!({ "configured": false, "available": false }),
        json!({
            "status": "ok",
            "runtime": { "ok": true, "channel_kind": "acp_session" },
            "adapter_environment": {
                "browser": { "active_source": "host_bridge", "status": "ok" },
                "services": { "den": { "status": "ok" } },
                "diagnostics": { "warnings": ["adapter warning"], "errors": [] }
            }
        }),
    );

    assert_eq!(payload["bear"]["slug"], "meta");
    assert_eq!(payload["runtime"]["state"], "running");
    assert_eq!(payload["session"]["id"], "sess-123");
    assert_eq!(payload["workspace"]["cwd"], "/workspace");
    assert_eq!(payload["browser"]["active_source"], "host_bridge");
    assert_eq!(payload["environment_variants"]["acp"]["status"], "ok");
    assert_eq!(payload["environment_variants"]["adapter"]["status"], "ok");
    assert_eq!(payload["diagnostics"]["warnings"][0], "adapter warning");
    assert!(payload["tools"]["available_den_tools"].is_array());
}

#[test]
fn privileged_descriptors_are_role_scoped() {
    let talk = names_for_role(BearAgentRole::Talk);
    assert!(talk.contains(DEN_TASK_WRITE_INTENT));
    assert!(talk.contains(DEN_SKILL_PROPOSE));
    assert!(!talk.contains(DEN_OBSERVATION_WRITE));
    assert!(!talk.contains(DEN_RUN_WRITE_RESULT));

    let pair = names_for_role(BearAgentRole::Pair);
    assert!(pair.contains(DEN_TASK_WRITE_INTENT));
    assert!(pair.contains(DEN_WORK_PLAN_UPDATE));
    assert!(pair.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
    assert!(pair.contains(DEN_SKILL_PROPOSE));
    assert!(!pair.contains(DEN_OBSERVATION_WRITE));
    assert!(!pair.contains(DEN_RUN_WRITE_RESULT));

    let curate = names_for_role(BearAgentRole::Curate);
    assert!(curate.contains(DEN_TASK_APPROVE_INTENT));
    assert!(curate.contains(DEN_TASK_REJECT_INTENT));
    assert!(curate.contains(DEN_CORE_WRITE_RESULT_SUMMARY));
    assert!(curate.contains(DEN_SKILL_APPROVE_PROPOSAL));
    assert!(curate.contains(DEN_SKILL_REJECT_PROPOSAL));
    assert!(curate.contains(DEN_SKILL_PROPOSE));
    assert!(!curate.contains(DEN_TASK_WRITE_INTENT));
    assert!(!curate.contains(DEN_OBSERVATION_WRITE));
    assert!(!curate.contains(DEN_RUN_WRITE_RESULT));

    let watch = names_for_role(BearAgentRole::Watch);
    assert!(watch.contains(DEN_OBSERVATION_WRITE));
    assert!(watch.contains(DEN_SKILL_PROPOSE));
    assert!(!watch.contains(DEN_WORK_PLAN_LIST));
    assert!(!watch.contains(DEN_WORK_PLAN_UPDATE));
    assert!(!watch.contains(DEN_TASK_WRITE_INTENT));
    assert!(!watch.contains(DEN_RUN_WRITE_RESULT));

    let work = names_for_role(BearAgentRole::Work);
    assert!(work.contains(DEN_RUN_WRITE_RESULT));
    assert!(work.contains(DEN_WORK_PLAN_LIST));
    assert!(work.contains(DEN_WORK_PLAN_UPDATE));
    assert!(!work.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
    assert!(work.contains(DEN_SKILL_PROPOSE));
    assert!(!work.contains(DEN_TASK_WRITE_INTENT));
    assert!(!work.contains(DEN_OBSERVATION_WRITE));
}

#[test]
fn all_descriptors_are_known_tools() {
    for descriptor in builtin_den_tool_descriptors() {
        assert!(
            is_builtin_den_tool(descriptor.name),
            "unknown descriptor name: {}",
            descriptor.name
        );
    }
}

#[test]
fn pair_has_web_memory_and_activity_tools() {
    let pair = names_for_role(BearAgentRole::Pair);
    assert!(pair.contains(DEN_CONVERSATION_SET_TITLE));
    assert!(pair.contains(DEN_WEB_FETCH));
    assert!(pair.contains(DEN_WEB_SEARCH));
    assert!(pair.contains(DEN_BEAR_ENVIRONMENT));
    assert!(pair.contains(DEN_SITUATION_GET));
    assert!(pair.contains(DEN_MEMORY_WRITE_ENTRY));
    assert!(pair.contains(DEN_MEMORY_STATUS));
    assert!(pair.contains(DEN_MEMORY_TREE));
    assert!(pair.contains(DEN_MEMORY_READ));
    assert!(pair.contains(DEN_MEMORY_SEARCH));
    assert!(pair.contains(DEN_WORK_PLAN_LIST));
    assert!(pair.contains(DEN_WORK_PLAN_GET_STATUS));
    assert!(pair.contains(DEN_WORK_PLAN_UPDATE));
    assert!(pair.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
    assert!(pair.contains(DEN_PLAN_MODE_ENTER));
    assert!(pair.contains(DEN_PLAN_MODE_STATUS));
    assert!(pair.contains(DEN_PLAN_MODE_RECORD_APPROVAL));
    assert!(pair.contains(DEN_PLAN_MODE_EXIT));
    assert!(pair.contains(DEN_PLAN_MODE_CANCEL));

    let talk = names_for_role(BearAgentRole::Talk);
    assert!(talk.contains(DEN_CONVERSATION_SET_TITLE));
    assert!(!talk.contains(DEN_WEB_FETCH));
    assert!(!talk.contains(DEN_WEB_SEARCH));
    assert!(!talk.contains(DEN_MEMORY_WRITE_ENTRY));
}

#[tokio::test]
async fn web_search_reports_missing_provider_config() {
    let config = Config::test_stub();
    let err = web_search_inner(None, &config, None, json!({ "query": "rust docs" }))
        .await
        .expect_err("missing provider should fail clearly");
    assert!(err.to_string().contains("DEN_SEARCH_PROVIDER"));
}

#[test]
fn role_authorization_rejects_disallowed_tools() {
    assert!(authorize_tool_for_role(DEN_TASK_WRITE_INTENT, BearAgentRole::Talk).is_ok());
    assert!(authorize_tool_for_role(DEN_TASK_WRITE_INTENT, BearAgentRole::Watch).is_err());
    assert!(authorize_tool_for_role(DEN_RUN_WRITE_RESULT, BearAgentRole::Work).is_ok());
    assert!(authorize_tool_for_role(DEN_RUN_WRITE_RESULT, BearAgentRole::Talk).is_err());
    assert!(authorize_tool_for_role(DEN_TASK_APPROVE_INTENT, BearAgentRole::Curate).is_ok());
    assert!(authorize_tool_for_role(DEN_TASK_APPROVE_INTENT, BearAgentRole::Pair).is_err());
    assert!(authorize_tool_for_role(DEN_SKILL_APPROVE_PROPOSAL, BearAgentRole::Curate).is_ok());
    assert!(authorize_tool_for_role(DEN_SKILL_APPROVE_PROPOSAL, BearAgentRole::Work).is_err());
    assert!(authorize_tool_for_role(DEN_WORK_PLAN_UPDATE, BearAgentRole::Pair).is_ok());
    assert!(authorize_tool_for_role(DEN_WORK_PLAN_UPDATE, BearAgentRole::Watch).is_err());
    assert!(authorize_tool_for_role(DEN_WORK_PLAN_REQUEST_HANDOFF, BearAgentRole::Work).is_err());
}
