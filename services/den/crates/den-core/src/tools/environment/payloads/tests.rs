    use super::{bear_environment_payload, session_info_payload};
    use crate::tools::descriptor::builtin_den_tool_descriptors_for_profile;
    use crate::tools::arguments::DenToolChannelContext;
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
            acp_session_id: Some("acp-test".to_string()),
            conversation_selection: Some("src/main.rs".to_string()),
            runtime_target: Some("repo:builder-bear".to_string()),
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            runtime: None,
            context_budget: None,
            request_id: None,
            channel: Default::default(),
        }
    }

    #[test]
    fn pair_session_info_context_fields_distinguish_role_contract_from_runtime() {
        let context = pair_context();
        let payload =
            session_info_payload(&context, BearProfile::Pair, None, 2, &json!({ "available": true }), &json!({ "status": "ok" }));

        assert_eq!(
            payload["role_contract_context"]["contract_label"],
            json!("Builder Bear")
        );
        assert_eq!(payload["role_contract_context"]["profile"], json!("pair"));
        assert_eq!(
            payload["runtime_context"]["active_bear_slug"],
            json!("test")
        );
        assert_eq!(
            payload["runtime_context"]["active_bear_authority"],
            json!("trusted_session")
        );
        assert_eq!(payload["context_composition_note"], json!("Role-contract context defines role behavior and style. Runtime context defines active Bear attachment, scope, attribution, workspace, and permissions for this session."));
        assert_eq!(payload["agent_context_summary"], json!("You are the pair-role collaborator operating under the Builder Bear role-contract context, currently attached to the test Bear runtime context."));
    }

    #[test]
    fn pair_session_info_includes_runtime_health_and_context_budget_defaults() {
        let context = pair_context();
        let payload =
            session_info_payload(&context, BearProfile::Pair, None, 2, &json!({ "available": true }), &json!({ "status": "ok" }));

        assert_eq!(payload["runtime"]["state"], json!("idle"));
        assert_eq!(payload["runtime"]["active_turn"]["present"], json!(false));
        assert_eq!(
            payload["runtime"]["active_turn"]["pending_obligations"],
            json!(0)
        );
        assert_eq!(
            payload["runtime"]["active_turn"]["pending_adapter_tools"],
            json!(0)
        );
        assert_eq!(
            payload["runtime"]["active_turn"]["pending_den_tools"],
            json!(0)
        );
        assert_eq!(payload["runtime"]["source"], json!("session_info_default"));
        assert_eq!(payload["context_budget"]["status"], json!("unavailable"));
        assert_eq!(
            payload["context_budget"]["source"],
            json!("den.session_info")
        );
    }

    #[test]
    fn pair_session_info_uses_context_runtime_health_when_available() {
        let mut context = pair_context();
        context.runtime = Some(json!({
            "state": "requires_action",
            "active_turn": {
                "present": true,
                "phase": "WaitingForObligations",
                "pending_obligations": 1,
                "pending_adapter_tools": 1,
                "pending_den_tools": 0,
                "pending_permissions": 0
            },
            "source": "acp_active_turn_registry"
        }));
        context.context_budget = Some(json!({
            "status": "estimated",
            "used_tokens": 1000,
            "remaining_tokens": 9000,
            "source": "test"
        }));
        let payload =
            session_info_payload(&context, BearProfile::Pair, None, 2, &json!({ "available": true }), &json!({ "status": "ok" }));

        assert_eq!(payload["runtime"]["state"], json!("requires_action"));
        assert_eq!(payload["runtime"]["active_turn"]["present"], json!(true));
        assert_eq!(
            payload["runtime"]["active_turn"]["pending_adapter_tools"],
            json!(1)
        );
        assert_eq!(
            payload["runtime"]["source"],
            json!("acp_active_turn_registry")
        );
        assert_eq!(payload["context_budget"]["status"], json!("estimated"));
        assert_eq!(payload["context_budget"]["source"], json!("test"));
    }

    #[test]
    fn chat_profile_exposes_memory_read_and_write_tools() {
        let names: Vec<_> = builtin_den_tool_descriptors_for_profile(BearProfile::Chat)
            .into_iter()
            .filter(|descriptor| descriptor.domain == "memory")
            .map(|descriptor| descriptor.provider_name)
            .collect();
        assert!(names.contains(&"memory_search".to_string()));
        assert!(names.contains(&"memory_read".to_string()));
        assert!(names.contains(&"memory_write_entry".to_string()));
        assert!(!names.contains(&"memory_list_proposals".to_string()));
    }

    #[test]
    fn chat_session_info_available_tools_match_memory_roster() {
        let context = DenToolInvocationContext {
            bear_id: uuid::Uuid::nil(),
            bear_slug: "meta".to_string(),
            binding_id: "agent-123".to_string(),
            profile: Some(BearProfile::Chat),
            user_id: 7,
            username: Some("gerwitz".to_string()),
            membership_role: Some("admin".to_string()),
            conversation_id: "conv-123".to_string(),
            session_id: "sess-123".to_string(),
            acp_session_id: None,
            conversation_selection: Some("conv-123".to_string()),
            runtime_target: Some("conv-123".to_string()),
            workspace_roots: Vec::new(),
            session_policy: None,
            activity: None,
            runtime: None,
            context_budget: None,
            request_id: None,
            channel: Default::default(),
        };
        let payload = session_info_payload(
            &context,
            BearProfile::Chat,
            None,
            2,
            &json!({ "available": true }),
            &json!({ "status": "ok" }),
        );
        let tools = payload["memory"]["available_tools"]
            .as_array()
            .expect("available_tools array");
        let names: Vec<_> = tools
            .iter()
            .filter_map(|value| value.as_str())
            .collect();
        assert!(names.contains(&"memory_search"));
        assert!(names.contains(&"memory_write_entry"));
        assert!(!names.contains(&"memory_list_proposals"));
    }

    #[test]
    fn bear_environment_payload_exposes_baseline_sections() {
        let context = DenToolInvocationContext {
            bear_id: uuid::Uuid::nil(),
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
            false,
            BearProfile::Pair,
            None,
            2,
            &json!({ "configured": false, "available": false }),
            &json!({ "status": "ok" }),
            &json!({
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
