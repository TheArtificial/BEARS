//! Pure payload builders for the `session_info` and `bear_environment` tools.
//!
//! Relocated from `den::core::tools::payloads`; these are stateless renderers over
//! the per-call context plus runtime-supplied snapshots (memory status, adapter
//! runtime). Identity comes in as the runtime-neutral [`CurrentUser`] DTO.

use serde_json::{json, Value};

use crate::BearProfile;

use crate::tools::{
    context::DenToolInvocationContext,
    descriptor::{
        builtin_den_tool_descriptors_for_profile, memory_tool_provider_names_for_profile,
    },
    identity::{role_is_bear_admin, CurrentUser},
    memory::source_client_session_id,
    support::{clean_optional, memory_read_scopes, memory_write_scopes},
    work_surface::infer_work_surface_hint,
};

fn trusted_workspace_from_context(
    context: &DenToolInvocationContext,
    adapter_runtime: Option<&Value>,
) -> Value {
    if let Some(trusted) = adapter_runtime
        .and_then(|value| value.get("trusted_workspace"))
        .cloned()
    {
        return trusted;
    }
    json!({
        "cwd": context.workspace_roots.first().cloned(),
        "roots": context.workspace_roots,
        "source": if context.workspace_roots.is_empty() { "none" } else { "trusted_session" },
    })
}

fn memory_context_layers(
    context: &DenToolInvocationContext,
    context_budget: &Value,
    memory_status: &Value,
    entities: &Value,
) -> Value {
    let durable_memory_status = if memory_status
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "available"
    } else {
        match memory_status.get("configured").and_then(Value::as_bool) {
            Some(true) => "degraded",
            Some(false) => "unavailable",
            None => "unknown",
        }
    };
    let memory_scope = format!("{}/", context.profile.unwrap_or(BearProfile::Pair).as_str());
    json!({
        "schema": "den.memory_context_layers.v1",
        "source": "den.session_info",
        "layers": [
            {
                "name": "conversation_context",
                "label": "Current conversation/context",
                "status": "available",
                "scope": "session",
                "lifetime": "transient",
                "mutability": "append_only_during_turn",
                "authority": "current_transcript_and_runtime",
                "next_surface": "conversation transcript / session_info.context_budget"
            },
            {
                "name": "context_budget",
                "label": "Context budget",
                "status": context_budget.get("status").and_then(Value::as_str).unwrap_or("unknown"),
                "scope": "session",
                "lifetime": "turn",
                "mutability": "runtime_reported",
                "authority": "provider_or_den_estimate",
                "next_surface": "session_info.context_budget"
            },
            {
                "name": "projected_memory",
                "label": "Prompt-projected memory",
                "status": "unknown",
                "scope": memory_scope.clone(),
                "lifetime": "prompt_projection",
                "mutability": "runtime_selected",
                "authority": "prompt_context",
                "count": "unknown",
                "reason": "Projection metadata is not wired into session_info yet.",
                "next_surface": "prompt memory blocks in model prompt / future projection diagnostic"
            },
            {
                "name": "recalled_memory",
                "label": "Turn-start recalled memory",
                "status": "unknown",
                "scope": memory_scope.clone(),
                "lifetime": "turn",
                "mutability": "runtime_selected",
                "authority": "recall_pipeline",
                "count": "unknown",
                "reason": "Recall passage metadata is not wired into session_info yet.",
                "next_surface": "memory_search / future recall diagnostic"
            },
            {
                "name": "durable_memory",
                "label": "Persistent semantic memory",
                "status": durable_memory_status,
                "scope": memory_scope,
                "lifetime": "durable_or_lifecycle_managed",
                "mutability": "explicit_tool_or_review_flow",
                "authority": "sqlite_memory_store",
                "next_surface": "session_info.memory.status / memory_status / memory_search"
            },
            {
                "name": "task_work_state",
                "label": "Task, Docket, and workplan state",
                "status": if context.activity.is_some() { "available" } else { "unknown" },
                "scope": "work_surface_or_session",
                "lifetime": "task_lifecycle",
                "mutability": "task_tools",
                "authority": "docket_and_task_list_surfaces",
                "next_surface": "activity / task-list tools"
            },
            {
                "name": "entity_context",
                "label": "Entity/work-surface orientation",
                "status": entities.get("status").and_then(Value::as_str).unwrap_or("unknown"),
                "scope": "work_surface_or_bear",
                "lifetime": "lifecycle_managed",
                "mutability": "entity_tools",
                "authority": "entity_orientation_surface",
                "next_surface": "session_info.entities / entity tools"
            },
            {
                "name": "tool_runtime_surface",
                "label": "Runtime instructions and callable tools",
                "status": "available",
                "scope": "turn",
                "lifetime": "turn",
                "mutability": "runtime_controlled",
                "authority": "system_developer_prompt_and_tool_descriptors",
                "next_surface": "tool descriptors / session_info.policy"
            }
        ]
    })
}

pub fn bear_environment_payload(
    context: &DenToolInvocationContext,
    role: BearProfile,
    current_user: Option<&CurrentUser>,
    member_count: i64,
    memory_status: &Value,
    entities: &Value,
    adapter_runtime: &Value,
) -> Value {
    let session_info = session_info_payload(
        context,
        role,
        current_user,
        member_count,
        memory_status,
        entities,
    );
    let runtime = session_info.get("runtime").cloned().unwrap_or_else(|| {
        json!({
            "state": "idle",
            "source": "bear_environment_default"
        })
    });
    let session = json!({
        "id": context.session_id,
        "client_session_id": source_client_session_id(context),
        "conversation_id": clean_optional(&context.conversation_id),
        "conversation_selection": context.conversation_selection,
        "runtime_target": context.runtime_target,
        "request_id": context.request_id,
        "channel": context.channel,
        "active_turn": runtime.get("active_turn").cloned().unwrap_or(Value::Null),
    });
    let mut workspace = trusted_workspace_from_context(context, Some(adapter_runtime));
    workspace["work_surface"] = infer_work_surface_hint(context, role)["work_surface"].clone();
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
            "status": if source_client_session_id(context).is_some() { "unavailable" } else { "unknown" },
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
            "status": if adapter_service.is_object() { "ok" } else if source_client_session_id(context).is_some() { "degraded" } else { "not_applicable" },
            "details": adapter_service,
        },
    });
    let has_client_session = source_client_session_id(context).is_some();
    let adapter_environment_status = adapter_runtime
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if has_client_session {
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
    let client_variant = if has_client_session {
        let client_runtime = adapter_runtime
            .get("runtime")
            .cloned()
            .unwrap_or_else(|| runtime.clone());
        json!({
            "status": "ok",
            "session": {
                "client_session_id": source_client_session_id(context),
                "conversation_selection": context.conversation_selection,
                "runtime_target": context.runtime_target,
            },
            "runtime": client_runtime,
            "permissions": context.session_policy,
        })
    } else {
        json!({ "status": "not_applicable" })
    };
    let adapter_variant = if has_client_session {
        if adapter_environment.is_object() {
            json!({
                "status": adapter_environment_status,
                "snapshot": adapter_environment,
            })
        } else {
            json!({
                "status": adapter_environment_status,
                "note": "Adapter enrichment is not available for this client session.",
            })
        }
    } else {
        json!({ "status": "not_applicable" })
    };
    let diagnostics_warnings = {
        let mut warnings = Vec::<Value>::new();
        if has_client_session && !adapter_environment.is_object() {
            warnings.push(json!(
                "Adapter enrichment is not available for this client session."
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
            "memory_runtime": "sqlite",
        },
        "session": session,
        "workspace": workspace,
        "tools": tools,
        "browser": browser,
        "services": services,
        "environment_variants": {
            "client": client_variant,
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
    entities: &Value,
) -> Value {
    let work_surface = infer_work_surface_hint(context, role);
    let workspace = trusted_workspace_from_context(context, None);
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
            "reason": "Provider context usage data is not wired into Den session_info yet",
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
    let context_layers = memory_context_layers(context, &context_budget, memory_status, entities);
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
            "workspace_root": workspace.get("cwd").cloned().unwrap_or(Value::Null),
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
            "relationship": "authenticated Armature token owner; memory entries and logs should attribute work to this human"
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
        "context_surfaces": context_layers,
        "model_experience": {
            "schema": "den.model_experience.memory_surfaces.v1",
            "source": "den.session_info",
            "guide": "docs/guides/bear-memory.md#model-experience",
            "rule": "Describe memory and context by explicit layer; say unknown or unavailable instead of guessing when a layer has no diagnostic data.",
            "next_surfaces": [
                "session_info.context_budget",
                "session_info.context_surfaces.layers",
                "session_info.memory.status",
                "memory_status",
                "memory_search",
                "task-list tools"
            ]
        },
        "session": {
            "conversation_id": context.conversation_id,
            "session_id": context.session_id,
            "client_session_id": context.client_session_id,
            "conversation_selection": context.conversation_selection,
            "runtime_target": context.runtime_target,
            "request_id": context.request_id,
            "channel": context.channel
        },
        "channel": context.channel,
        "workspace": workspace,
        "work_surface": work_surface,
        "entities": entities,
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
mod tests;
