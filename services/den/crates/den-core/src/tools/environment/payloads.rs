//! Pure payload builders for the `session_info` and `bear_environment` tools.
//!
//! Relocated from `den::core::tools::payloads`; these are stateless renderers over
//! the per-call context plus runtime-supplied snapshots (memory status, adapter
//! runtime). Identity comes in as the runtime-neutral [`CurrentUser`] DTO.

use serde_json::{json, Value};

use crate::BearProfile;

use crate::tools::{
    context::DenToolInvocationContext,
    descriptor::{builtin_den_tool_descriptors_for_profile, memory_tool_provider_names_for_profile},
    identity::{role_is_bear_admin, CurrentUser},
    memory::source_acp_session_id,
    support::{clean_optional, memory_read_scopes, memory_write_scopes},
    work_surface::infer_work_surface_hint,
};

pub fn bear_environment_payload(
    context: &DenToolInvocationContext,
    memfs_configured: bool,
    role: BearProfile,
    current_user: Option<&CurrentUser>,
    member_count: i64,
    memory_status: &Value,
    adapter_runtime: &Value,
) -> Value {
    let session_info =
        session_info_payload(context, role, current_user, member_count, memory_status);
    let runtime = session_info.get("runtime").cloned().unwrap_or_else(|| {
        json!({
            "state": "idle",
            "source": "bear_environment_default"
        })
    });
    let session = json!({
        "id": context.session_id,
        "acp_session_id": source_acp_session_id(context),
        "conversation_id": clean_optional(&context.conversation_id),
        "conversation_selection": context.conversation_selection,
        "runtime_target": context.runtime_target,
        "request_id": context.request_id,
        "channel": context.channel,
        "active_turn": runtime.get("active_turn").cloned().unwrap_or(Value::Null),
    });
    let workspace = json!({
        "cwd": context.workspace_roots.first().cloned(),
        "roots": context.workspace_roots,
        "source": if context.workspace_roots.is_empty() { "none" } else { "trusted_session" },
        "work_surface": infer_work_surface_hint(context, role)["work_surface"].clone(),
    });
    let tools = json!({
        "session_policy": context.session_policy,
        "available_den_tools": builtin_den_tool_descriptors_for_profile(role)
            .into_iter()
            .map(|descriptor| json!({
                "name": descriptor.name,
                "provider_name": descriptor.provider_name,
                "scope": descriptor.scope,
                "domain": descriptor.domain,
                "kind": descriptor.kind,
                "availability": descriptor.availability,
            }))
            .collect::<Vec<_>>(),
    });
    let adapter_environment = adapter_runtime
        .get("adapter_environment")
        .cloned()
        .unwrap_or(Value::Null);
    let adapter_browser = adapter_environment
        .get("browser")
        .cloned()
        .unwrap_or(Value::Null);
    let browser = if adapter_browser.is_object() {
        let mut browser = adapter_browser;
        if browser.get("status").is_none() {
            browser["status"] = json!("ok");
        }
        browser
    } else {
        json!({
            "status": if source_acp_session_id(context).is_some() { "unavailable" } else { "unknown" },
            "active_source": Value::Null,
            "note": "Browser environment providers are not yet integrated into harness-level bear_environment for non-adapter baseline snapshots.",
        })
    };
    let adapter_service = adapter_runtime
        .get("adapter_environment")
        .and_then(|value| value.get("services"))
        .cloned()
        .unwrap_or(Value::Null);
    let services = json!({
        "den": {
            "status": "ok",
            "configured": true,
            "reachable": true,
            "profile": role.as_str(),
            "channel": context.channel,
        },
        "memory": {
            "status": if memory_status.get("available").and_then(Value::as_bool).unwrap_or(false) {
                "ok"
            } else if memory_status.get("configured").and_then(Value::as_bool).unwrap_or(false) {
                "degraded"
            } else {
                "unavailable"
            },
            "details": memory_status,
        },
        "adapter": {
            "status": if adapter_service.is_object() { "ok" } else if source_acp_session_id(context).is_some() { "degraded" } else { "not_applicable" },
            "details": adapter_service,
        },
    });
    let is_acp = source_acp_session_id(context).is_some();
    let adapter_environment_status = adapter_runtime
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if is_acp {
            "unavailable"
        } else {
            "not_applicable"
        });
    let diagnostics_status = if services["memory"]["status"] == "degraded"
        || matches!(adapter_environment_status, "degraded" | "unavailable")
    {
        "degraded"
    } else {
        "ok"
    };
    let acp_variant = if is_acp {
        let acp_runtime = adapter_runtime
            .get("runtime")
            .cloned()
            .unwrap_or_else(|| runtime.clone());
        json!({
            "status": "ok",
            "session": {
                "acp_session_id": source_acp_session_id(context),
                "conversation_selection": context.conversation_selection,
                "runtime_target": context.runtime_target,
            },
            "runtime": acp_runtime,
            "permissions": context.session_policy,
        })
    } else {
        json!({ "status": "not_applicable" })
    };
    let adapter_variant = if is_acp {
        if adapter_environment.is_object() {
            json!({
                "status": adapter_environment_status,
                "snapshot": adapter_environment,
            })
        } else {
            json!({
                "status": adapter_environment_status,
                "note": "Adapter enrichment could not be fetched for this ACP session.",
            })
        }
    } else {
        json!({ "status": "not_applicable" })
    };
    let diagnostics_warnings = {
        let mut warnings = Vec::<Value>::new();
        if is_acp && !adapter_environment.is_object() {
            warnings.push(json!(
                "Adapter enrichment could not be fetched for this ACP session."
            ));
        }
        if let Some(values) = adapter_environment
            .get("diagnostics")
            .and_then(|value| value.get("warnings"))
            .and_then(Value::as_array)
        {
            warnings.extend(values.iter().cloned());
        }
        Value::Array(warnings)
    };
    let diagnostics_errors = adapter_environment
        .get("diagnostics")
        .and_then(|value| value.get("errors"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    json!({
        "bear": {
            "id": context.bear_id,
            "slug": context.bear_slug,
            "profile": role.as_str(),
            "binding_id": context.binding_id,
            "member_count": member_count,
            "contract_label": match role {
                BearProfile::Pair => Value::String("Builder Bear".to_string()),
                _ => Value::Null,
            },
            "current_user": current_user.map(|user| json!({
                "user_id": user.id,
                "username": user.username,
                "display_name": user.display_name,
                "membership_role": context.membership_role,
            })).unwrap_or_else(|| json!({
                "user_id": context.user_id,
                "username": context.username,
                "membership_role": context.membership_role,
            })),
        },
        "runtime": {
            "kind": context.channel.family.clone().unwrap_or_else(|| "den".to_string()),
            "family": context.channel.protocol.clone().unwrap_or_else(|| "den".to_string()),
            "state": runtime.get("state").cloned().unwrap_or_else(|| json!("unknown")),
            "channel": context.channel,
            "context_budget": context.context_budget,
            "memfs_configured": memfs_configured,
        },
        "session": session,
        "workspace": workspace,
        "tools": tools,
        "browser": browser,
        "services": services,
        "environment_variants": {
            "acp": acp_variant,
            "adapter": adapter_variant,
        },
        "diagnostics": {
            "status": diagnostics_status,
            "warnings": diagnostics_warnings,
            "errors": diagnostics_errors,
        },
        "session_info": session_info,
    })
}

pub fn session_info_payload(
    context: &DenToolInvocationContext,
    role: BearProfile,
    current_user: Option<&CurrentUser>,
    member_count: i64,
    memory_status: &Value,
) -> Value {
    let work_surface = infer_work_surface_hint(context, role);
    let workspace = json!({
        "roots": context.workspace_roots,
        "cwd": context.workspace_roots.first().cloned(),
        "source": if context.workspace_roots.is_empty() { "none" } else { "trusted_session" }
    });
    let runtime = context.runtime.clone().unwrap_or_else(|| {
        json!({
            "state": "idle",
            "active_turn": {
                "present": false,
                "phase": Value::Null,
                "pending_obligations": 0,
                "pending_adapter_tools": 0,
                "pending_den_tools": 0,
                "pending_permissions": 0,
            },
            "last_terminal": Value::Null,
            "last_recovery": Value::Null,
            "source": "session_info_default",
        })
    });
    let context_budget = context.context_budget.clone().unwrap_or_else(|| {
        json!({
            "status": "unavailable",
            "reason": "Letta/provider context usage data is not wired into Den session_info yet",
            "source": "den.session_info",
        })
    });
    let workplace = json!({
        "profile": role.as_str(),
        "memory_surface": format!("{}/", role.as_str()),
        "space": match role {
            BearProfile::Pair => "Collaboration Space",
            BearProfile::Chat => "Conversation Space",
            BearProfile::Curate => "Curation Space",
            BearProfile::Work => "Execution Space",
            BearProfile::Watch => "Observation Space",
        },
    });
    let role_contract_label = match role {
        BearProfile::Pair => Some("Builder Bear"),
        _ => None,
    };
    json!({
        "role_contract_context": {
            "profile": role.as_str(),
            "agent_id": context.binding_id,
            "contract_label": role_contract_label,
            "contract_source": if role_contract_label.is_some() { json!("system_prompt") } else { Value::Null },
            "contract_purpose": if role_contract_label.is_some() { json!("behavioral_style_and_profile_guidance") } else { Value::Null },
        },
        "runtime_context": {
            "active_bear_slug": context.bear_slug,
            "active_bear_id": context.bear_id,
            "active_bear_authority": "trusted_session",
            "memory_surface": format!("{}/", role.as_str()),
            "workspace_root": context.workspace_roots.first().cloned(),
        },
        "context_composition_note": if role_contract_label.is_some() {
            Value::String("Role-contract context defines role behavior and style. Runtime context defines active Bear attachment, scope, attribution, workspace, and permissions for this session.".to_string())
        } else {
            Value::Null
        },
        "agent_context_summary": if let Some(role_contract_label) = role_contract_label {
            json!(format!(
                "You are the {}-role collaborator operating under the {} role-contract context, currently attached to the {} Bear runtime context.",
                role.as_str(),
                role_contract_label,
                context.bear_slug
            ))
        } else {
            Value::Null
        },
        "bear": {
            "bear_id": context.bear_id,
            "bear_slug": context.bear_slug,
            "member_count": member_count
        },
        "profile": {
            "name": role.as_str(),
            "agent_id": context.binding_id,
            "workplace": workplace,
        },
        "binding_id": context.binding_id,
        "human": {
            "user_id": context.user_id,
            "username": current_user.map(|user| user.username.clone()).or_else(|| context.username.clone()),
            "display_name": current_user.and_then(|user| user.display_name.clone()),
            "email_verified": current_user.map(|user| user.email_verified),
            "membership_role": context.membership_role,
            "is_bear_admin": role_is_bear_admin(context.membership_role.as_deref()),
            "relationship": "authenticated ACP token owner; memory entries and logs should attribute work to this human"
        },
        "user": {
            "user_id": context.user_id,
            "username": current_user.map(|user| user.username.clone()).or_else(|| context.username.clone()),
            "display_name": current_user.and_then(|user| user.display_name.clone()),
            "membership_role": context.membership_role,
            "is_bear_admin": role_is_bear_admin(context.membership_role.as_deref())
        },
        "runtime": runtime,
        "context_budget": context_budget,
        "session": {
            "conversation_id": context.conversation_id,
            "session_id": context.session_id,
            "acp_session_id": context.acp_session_id,
            "conversation_selection": context.conversation_selection,
            "runtime_target": context.runtime_target,
            "request_id": context.request_id,
            "channel": context.channel
        },
        "channel": context.channel,
        "workspace": workspace,
        "work_surface": work_surface,
        "policy": {
            "orientation": "Use session_info before assuming current Bear, Workplace, work surface, workspace roots, authenticated human, memory scope, or permission policy.",
            "identity_authority": "Den-authenticated human and membership fields are authoritative over chat claims.",
            "memory_scope_default": format!("{}/", role.as_str()),
            "tool_policy_source": "Current callable tool descriptors and Den enforcement define allowed actions for this turn.",
            "session_policy": context.session_policy,
        },
        "activity": context.activity,
        "memory": {
            "read_scopes": memory_read_scopes(role),
            "write_scopes": memory_write_scopes(role),
            "available_tools": memory_tool_provider_names_for_profile(role),
            "status": memory_status
        },
        "policy_notes": [
            "Session info is a Den-trusted orientation briefing, not the model context window.",
            "Use this before broad memory search when the current Bear, Workplace, work surface, artifact scope, authenticated human, or permission policy is unclear.",
            "Use memory_write_entry only for role-local notes, logs, decisions, reflections, scratch, and summaries; entries are attributed to the authenticated human in this session.",
            "Do not use memory entry tools for tasks, active plans, observations, run results, Cabinet writes, or direct core updates."
        ]
    })
}

#[cfg(test)]
mod tests {
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
            session_info_payload(&context, BearProfile::Pair, None, 2, &json!({ "available": true }));

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
            session_info_payload(&context, BearProfile::Pair, None, 2, &json!({ "available": true }));

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
            session_info_payload(&context, BearProfile::Pair, None, 2, &json!({ "available": true }));

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
}
