use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        agent_loop::key_memory_projection::{project_key_memory, KeyMemoryProjectionInput},
        bears::{model::BearProfile, Bear},
        memory::{
            store::{append_memory_record, LogicalMemoryPath},
            MemoryStoreManager,
        },
        tools::work_surface::WorkSurfaceSessionHints,
    },
};

fn legacy_test_bear(bear_id: Uuid) -> Bear {
    let now = OffsetDateTime::now_utc();
    Bear {
        id: bear_id,
        slug: "test-bear".to_string(),
        name: "Test Bear".to_string(),
        description: String::new(),
        default_model: None,
        tools_enabled: None,
        letta_agent_type: None,
        letta_tool_ids: sqlx::types::Json(Vec::new()),
        runtime_plan: None,
        context_profile: None,
        memfs_repo_path: None,
        provisioning_version: 1,
        system_prompt: "You are a test bear.".to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn noop_pg_pool() -> sqlx::PgPool {
    sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop")
        .expect("lazy pool")
}

#[tokio::test]
async fn projects_shared_identity_anchors_without_work_surface() {
    let bear_id = Uuid::new_v4();
    let bear = legacy_test_bear(bear_id);
    let mut config = Config::test_stub();
    config.agent_runtime_mode = crate::config::AgentRuntimeMode::Native;
    config.bear_sqlite_data_dir = format!("/tmp/bears-kmp-{}", Uuid::new_v4());
    let stores = MemoryStoreManager::new(&config);
    let store = stores.store_for_bear(bear.id).await.expect("store");
    append_memory_record(
        &store,
        &LogicalMemoryPath::from_logical_path("core/bear-overview.md"),
        "overview",
        "curate",
        None,
        "Charter summary",
        &serde_json::json!({}),
    )
    .await
    .expect("append");

    let pool = noop_pg_pool();
    let result = project_key_memory(KeyMemoryProjectionInput {
        pool: &pool,
        stores: &stores,
        bear: &bear,
        role: BearProfile::Pair,
        conversation_id: "den-conv-test",
        session_hints: WorkSurfaceSessionHints::default(),
        work_surface_status_override: None,
    })
    .await
    .expect("project");

    assert!(result.rendered_text.contains("# Projected memory"));
    assert!(result.rendered_text.contains("Charter summary"));
    assert!(result.rendered_text.contains("## Shared anchors"));
    assert!(!result.rendered_text.contains("## Work surface:"));
}

#[tokio::test]
async fn candidate_work_surface_requires_canonical_anchor_for_tier2() {
    let bear_id = Uuid::new_v4();
    let bear = legacy_test_bear(bear_id);
    let mut config = Config::test_stub();
    config.agent_runtime_mode = crate::config::AgentRuntimeMode::Native;
    config.bear_sqlite_data_dir = format!("/tmp/bears-kmp-anchor-{}", Uuid::new_v4());
    let stores = MemoryStoreManager::new(&config);
    let hints = WorkSurfaceSessionHints {
        workspace_roots: vec!["/workspace/bears-monorepo".to_string()],
        ..Default::default()
    };
    let pool = noop_pg_pool();

    let without_anchor = project_key_memory(KeyMemoryProjectionInput {
        pool: &pool,
        stores: &stores,
        bear: &bear,
        role: BearProfile::Pair,
        conversation_id: "den-conv-test",
        session_hints: hints.clone(),
        work_surface_status_override: Some("candidate"),
    })
    .await
    .expect("project");
    assert!(!without_anchor.rendered_text.contains("## Work surface:"));
    assert_eq!(
        without_anchor.diagnostic["omitted_because_no_surface"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "tier2:slug=bears-monorepo:anchor_required"
    );

    let store = stores.store_for_bear(bear.id).await.expect("store");
    append_memory_record(
        &store,
        &LogicalMemoryPath::from_logical_path("core/work_surfaces/bears-monorepo/overview.md"),
        "overview",
        "curate",
        None,
        "Canonical overview",
        &serde_json::json!({}),
    )
    .await
    .expect("append overview");

    let with_anchor = project_key_memory(KeyMemoryProjectionInput {
        pool: &pool,
        stores: &stores,
        bear: &bear,
        role: BearProfile::Pair,
        conversation_id: "den-conv-test",
        session_hints: hints,
        work_surface_status_override: Some("candidate"),
    })
    .await
    .expect("project");
    assert!(with_anchor.rendered_text.contains("## Work surface: bears-monorepo"));
    assert!(with_anchor.rendered_text.contains("Canonical overview"));
}

#[tokio::test]
async fn resolved_work_surface_includes_tier2_without_prior_anchor_proof() {
    let bear_id = Uuid::new_v4();
    let bear = legacy_test_bear(bear_id);
    let mut config = Config::test_stub();
    config.agent_runtime_mode = crate::config::AgentRuntimeMode::Native;
    config.bear_sqlite_data_dir = format!("/tmp/bears-kmp-resolved-{}", Uuid::new_v4());
    let stores = MemoryStoreManager::new(&config);
    let store = stores.store_for_bear(bear.id).await.expect("store");
    append_memory_record(
        &store,
        &LogicalMemoryPath::from_logical_path("core/work_surfaces/my-app/decisions.md"),
        "decisions",
        "curate",
        None,
        "Use SQLite",
        &serde_json::json!({}),
    )
    .await
    .expect("append");

    let pool = noop_pg_pool();
    let result = project_key_memory(KeyMemoryProjectionInput {
        pool: &pool,
        stores: &stores,
        bear: &bear,
        role: BearProfile::Pair,
        conversation_id: "den-conv-test",
        session_hints: WorkSurfaceSessionHints {
            workspace_roots: vec!["/workspace/my-app".to_string()],
            ..Default::default()
        },
        work_surface_status_override: Some("resolved"),
    })
    .await
    .expect("project");

    assert!(result.rendered_text.contains("## Work surface: my-app"));
    assert!(result.rendered_text.contains("Use SQLite"));
}
