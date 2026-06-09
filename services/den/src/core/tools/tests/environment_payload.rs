use crate::{
    config::Config,
    core::{
        bears::BearProfile,
        tools::{arguments::DenToolChannelContext, payloads::bear_environment_payload, session::DenToolInvocationContext},
    },
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn bear_environment_payload_exposes_baseline_sections() {
    let context = DenToolInvocationContext {
        bear_id: Uuid::nil(),
        bear_slug: "meta".to_string(),
        binding_id: "agent-123".to_string(),
        profile: Some(BearProfile::Pair),
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
        BearProfile::Pair,
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
