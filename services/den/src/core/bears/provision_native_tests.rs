use uuid::Uuid;

use crate::config::Config;
use crate::core::bears::{
    db::{create_bear, get_bear_profile_binding, BearParams},
    model::BearProfile,
    provision::{provision_bear_if_configured, reconcile_bear_native},
};

#[sqlx::test]
async fn provision_bear_native_creates_den_native_bindings(
    pool: sqlx::PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::test_stub();
    config.bear_sqlite_data_dir = format!("/tmp/bears-provision-native-{}", Uuid::new_v4());

    let bear_id = create_bear(
        &pool,
        BearParams {
            slug: "native-provision-bear",
            name: "Native Provision Bear",
            description: "test",
            system_prompt: "You are a concise test bear.",
            default_model: Some("gpt-4.1"),
            tools_enabled: None,
            letta_agent_type: None,
            letta_tool_ids: sqlx::types::Json(vec![]),
            context_profile: None,
        },
    )
    .await?;

    provision_bear_if_configured(&pool, &config, bear_id).await?;

    for role in BearProfile::ALL {
        let row = get_bear_profile_binding(&pool, bear_id, role)
            .await?
            .expect("binding row should exist");
        assert_eq!(row.provisioning_status, "ready");
        assert!(
            row.binding_id.starts_with("den-native:"),
            "expected den-native binding id, got {}",
            row.binding_id
        );
        assert!(row.letta_agent_id.as_deref().unwrap_or("").trim().is_empty());
        assert!(row.config_hash.is_some());
    }

    let summary = reconcile_bear_native(&pool, &config, bear_id).await?;
    assert_eq!(summary.synced_count(), BearProfile::ALL.len());

    Ok(())
}
