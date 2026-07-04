    use super::infer_work_surface_hint;
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
        assert_eq!(payload["work_surface"]["reference_candidates"], json!([]));
    }
