use super::{infer_work_surface_hint, WorkSurfaceSessionAnchor};
use crate::tools::context::DenToolInvocationContext;
use crate::BearProfile;
use serde_json::json;

fn pair_context() -> DenToolInvocationContext {
    DenToolInvocationContext {
        bear_id: uuid::Uuid::nil(),
        bear_slug: "test".to_string(),
        binding_id: "agent".to_string(),
        profile: Some(BearProfile::Pair),
        user_id: 1,
        username: Some("tester".to_string()),
        membership_role: None,
        conversation_id: "conv-test".to_string(),
        session_id: "sess-test".to_string(),
        work_run_id: None,
        client_session_id: Some("client-test".to_string()),
        conversation_selection: Some("src/main.rs".to_string()),
        runtime_target: Some("repo:builder-bear".to_string()),
        workspace_roots: vec!["/workspace".to_string()],
        session_policy: None,
        activity: None,
        runtime: None,
        context_budget: None,
        projected_memory: None,
        recalled_memory: None,
        request_id: None,
        channel: Default::default(),
    }
}

#[test]
fn infer_work_surface_hint_surfaces_trusted_candidates() {
    let payload = infer_work_surface_hint(&pair_context(), BearProfile::Pair);
    assert_eq!(payload["workplace"]["profile"], json!("pair"));
    assert_eq!(payload["workplace"]["memory_surface"], json!("pair/"));
    assert_eq!(payload["work_surface"]["status"], json!("candidate"));
    assert_eq!(payload["work_surface"]["confidence"], json!("medium"));
    assert_eq!(payload["work_surface"]["needs_user_confirmation"], json!(false));
    assert_eq!(
        payload["work_surface"]["agent_guidance"]["may_state_assumption"],
        json!(true)
    );
    let candidates = payload["work_surface"]["reference_candidates"]
        .as_array()
        .expect("reference candidates array");
    assert!(candidates
        .iter()
        .any(|item| item["kind"] == json!("runtime_target")));
    assert!(candidates
        .iter()
        .any(|item| item["kind"] == json!("conversation_selection")));
    assert!(candidates
        .iter()
        .any(|item| item["kind"] == json!("workspace_root")));
}

#[test]
fn infer_work_surface_hint_reports_unresolved_without_trusted_candidates() {
    let mut context = pair_context();
    context.runtime_target = None;
    context.conversation_selection = None;
    context.workspace_roots.clear();

    let payload = infer_work_surface_hint(&context, BearProfile::Pair);
    assert_eq!(payload["work_surface"]["status"], json!("unresolved"));
    assert_eq!(payload["work_surface"]["confidence"], json!("none"));
    assert_eq!(payload["work_surface"]["needs_user_confirmation"], json!(false));
    assert_eq!(
        payload["work_surface"]["agent_guidance"]["may_state_assumption"],
        json!(false)
    );
    assert_eq!(payload["work_surface"]["reference_candidates"], json!([]));
}

#[test]
fn accepts_only_resolved_or_confirmed_typed_session_anchors() {
    let resolved = json!({
        "work_surface_anchor": {
            "surface_id": "00000000-0000-0000-0000-000000000001",
            "status": "resolved"
        }
    });
    assert_eq!(
        WorkSurfaceSessionAnchor::from_adapter_environment(Some(&resolved))
            .expect("resolved anchor")
            .surface_id,
        uuid::Uuid::from_u128(1)
    );

    for invalid in [
        json!({"work_surface_anchor": {"surface_id": "00000000-0000-0000-0000-000000000001", "status": "candidate"}}),
        json!({"work_surface_anchor": {"surface_id": "not-a-uuid", "status": "confirmed"}}),
        json!({"work_surface_anchor": {"status": "confirmed"}}),
    ] {
        assert!(WorkSurfaceSessionAnchor::from_adapter_environment(Some(&invalid)).is_none());
    }
}
