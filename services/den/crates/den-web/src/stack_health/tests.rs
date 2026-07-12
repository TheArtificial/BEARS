use super::{
    bifrost_management_auth_config_check_from_value, count_usable_bifrost_models, CheckState,
};

#[test]
fn bifrost_model_count_filters_routing_wildcards() {
    let value = serde_json::json!({
        "data": [
            { "id": "openai/*" },
            { "id": "*" },
            { "id": "openai/gpt-5.5" },
            { "id": "openai/gpt-4.1" }
        ]
    });

    assert_eq!(count_usable_bifrost_models(&value), (2, 2));
}

#[test]
fn bifrost_model_count_treats_wildcard_only_as_no_usable_models() {
    let value = serde_json::json!({
        "data": [
            { "id": "openai/*" },
            { "id": "*" }
        ]
    });

    assert_eq!(count_usable_bifrost_models(&value), (0, 2));
}

#[test]
fn bifrost_management_auth_config_accepts_enabled_top_level_auth() {
    let check = bifrost_management_auth_config_check_from_value(&serde_json::json!({
        "auth_config": { "is_enabled": true }
    }));

    assert_eq!(check.state, CheckState::Ok);
    assert!(check.detail.contains("auth_config.is_enabled=true"));
}

#[test]
fn bifrost_management_auth_config_flags_stale_disabled_runtime_config() {
    let check = bifrost_management_auth_config_check_from_value(&serde_json::json!({
        "auth_config": { "is_enabled": false }
    }));

    assert_eq!(check.state, CheckState::Fail);
    assert!(check.detail.contains("auth_config.is_enabled=false"));
    assert!(check.detail.contains("/app/data/config.db"));
}

#[test]
fn bifrost_management_auth_config_accepts_enabled_governance_auth() {
    let check = bifrost_management_auth_config_check_from_value(&serde_json::json!({
        "governance": { "auth_config": { "is_enabled": true } }
    }));

    assert_eq!(check.state, CheckState::Ok);
    assert!(check.detail.contains("auth_config.is_enabled=true"));
}
