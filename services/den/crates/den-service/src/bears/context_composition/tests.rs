    use super::*;
    use sqlx::types::Json;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn test_bear(profile: Option<BearContextProfile>) -> Bear {
        Bear {
            id: Uuid::nil(),
            slug: "builder".to_string(),
            name: "Builder Bear".to_string(),
            description: String::new(),
            default_model: Some("openai/gpt-4o".to_string()),
            tools_enabled: None,
            letta_agent_type: None,
            letta_tool_ids: Json(Vec::new()),
            runtime_plan: None,
            context_profile: profile
                .as_ref()
                .map(context_profile_to_json)
                .transpose()
                .unwrap(),
            memfs_repo_path: None,
            provisioning_version: 1,
            system_prompt: "legacy prompt".to_string(),
            birthday: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn legacy_bear_uses_system_prompt() {
        let bear = test_bear(None);
        let composed = compose_role_context(&bear, BearProfile::Chat, None).unwrap();
        assert!(composed.is_legacy);
        assert_eq!(composed.composed_prompt, "legacy prompt");
    }

    #[test]
    fn composed_bear_includes_layers_in_order() {
        let profile = BearContextProfile {
            composition_version: CONTEXT_PROFILE_VERSION,
            template_id: Some("software_product_builder".to_string()),
            template_version: Some("1".to_string()),
            role_contract_version: Some(DEFAULT_ROLE_CONTRACT_VERSION.to_string()),
            role_contracts: default_role_contracts_for_bear("Builder Bear"),
            user_steering: "Prefer concise plans.".to_string(),
            bear_context: "The user builds BEARS.".to_string(),
            starter_prompts: vec![],
            first_task: None,
        };
        let bear = test_bear(Some(profile));
        let composed =
            compose_role_context(&bear, BearProfile::Pair, Some("Runtime now.")).unwrap();
        assert!(!composed.is_legacy);
        assert!(composed.composed_prompt.contains("# Den baseline"));
        assert!(composed
            .composed_prompt
            .contains("# Space instructions: Collaboration Space"));
        assert!(composed.composed_prompt.contains("# User steering"));
        assert!(composed.composed_prompt.contains("# Bear context"));
        assert!(composed
            .composed_prompt
            .contains("# Runtime/thread context"));
    }

    #[test]
    fn managed_prompt_renders_compile_time_template_fields() {
        let mut profile = BearContextProfile {
            composition_version: CONTEXT_PROFILE_VERSION,
            template_id: None,
            template_version: None,
            role_contract_version: Some(DEFAULT_ROLE_CONTRACT_VERSION.to_string()),
            role_contracts: default_role_contracts_for_bear("{{ bear_name }}"),
            user_steering: "Prefer concise plans for {{ bear_name }}.".to_string(),
            bear_context: "Slug: {{ bear_slug }}.".to_string(),
            starter_prompts: vec![],
            first_task: None,
        };
        profile.role_contracts.chat = "Speak as {{ bear_name }}.".to_string();
        let bear = test_bear(Some(profile));
        let composed = render_managed_role_prompt(&bear, BearProfile::Chat, None).unwrap();
        assert!(composed.contains("Speak as Builder Bear."));
        assert!(composed.contains("Prefer concise plans for Builder Bear."));
        assert!(composed.contains("Slug: builder."));
    }

    #[test]
    fn managed_prompt_rejects_turn_time_template_fields() {
        let profile = BearContextProfile {
            composition_version: CONTEXT_PROFILE_VERSION,
            template_id: None,
            template_version: None,
            role_contract_version: Some(DEFAULT_ROLE_CONTRACT_VERSION.to_string()),
            role_contracts: RoleContracts {
                chat: "Today is {{ current_date }}.".to_string(),
                ..default_role_contracts_for_bear("Builder Bear")
            },
            user_steering: String::new(),
            bear_context: String::new(),
            starter_prompts: vec![],
            first_task: None,
        };
        let bear = test_bear(Some(profile));
        let err = render_managed_role_prompt(&bear, BearProfile::Chat, None).unwrap_err();
        assert!(err.to_string().contains("failed to render"));
    }

    #[test]
    fn pair_role_contract_includes_plan_request_guidance() {
        let pair_contract = default_role_contracts_for_bear("Builder Bear").pair;
        assert!(pair_contract.contains(
            "When the user asks to make, create, draft, update, or track a plan or task list"
        ));
        assert!(pair_contract
            .contains("prefer planning-state tools when the current runtime makes them available"));
        assert!(pair_contract
            .contains("Do not write active plans or ephemeral progress to durable memory"));
    }
