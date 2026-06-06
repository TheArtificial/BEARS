use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        acp_plan_mode::{self, AcpPlanModeRequestedBy, EnterPlanModeParams, SubmitPlanModeParams},
        acp_sessions,
        bears::{db as bears_db, db::role_is_bear_admin, BearAgentRole},
        memory_manager_head::{
            append_markdown_section, fetch_memfs_role_memory_file, fetch_memfs_role_memory_status,
            fetch_memfs_role_memory_tree, fetch_memfs_role_plan_artifacts,
            search_memfs_role_memory, write_memfs_core_update, write_memfs_role_memory_entry,
            MemfsCoreUpdateRequest, MemfsWriteRoleMemoryEntryRequest,
        },
        conversation_events::{
            memory_proposal_resolved_projection, memory_review_requested_projection,
            project_to_conversation, ProjectionProvenance, ProjectionSource,
        },
        memory_proposals::{self, CreateMemoryProposal},
        prompt_memory_block_store::{
            archive_conflicting_prompt_memory_blocks,
            archive_prompt_memory_blocks_superseded_by, list_prompt_memory_blocks_for_bear_role,
            patch_prompt_memory_block, upsert_prompt_memory_block, PromptMemoryBlockPatch,
            PromptMemoryBlockWrite,
        },
        prompt_memory_blocks::{
            PromptMemoryBlockScope, PromptMemoryBlockState, PromptMemoryBlockType,
        },
        turn_state, user,
        work_plans::{
            self, WorkPlanListFilter, WorkPlanLookup, WorkPlanStatus, WorkPlanUpdate,
            WorkPlanUpsert, WorkPlanVisibility,
        },
    },
    errors::CustomError,
};

pub(crate) fn plan_mode_workplan_payload(row: &acp_plan_mode::AcpPlanModeSessionRow) -> Value {
    turn_state::turn_state_from_sources(
        &crate::core::acp_tools::AcpResolvedSessionPolicy {
            mode_label: if row.state == "approved" {
                "Write"
            } else {
                "Plan"
            },
            tool_enablement: if row.state == "approved" {
                crate::core::acp_tools::AcpToolEnablementState::AllTools
            } else {
                crate::core::acp_tools::AcpToolEnablementState::ReadOnly
            },
            plan_mode_state: Some(row.state.clone()),
        },
        Some(row),
        None,
    )["workplan"]
        .clone()
}

pub(crate) fn no_active_workplan_payload() -> Value {
    json!({
        "domain": "workplan",
        "plan_id": Value::Null,
        "id": Value::Null,
        "state": "inactive",
        "approval_status": "inactive",
        "raw_state": Value::Null,
        "submitted_plan_present": false,
        "artifact_path": Value::Null,
        "title": Value::Null,
        "summary": Value::Null,
        "execution_unlocked": false,
    })
}

pub(crate) fn activity_payload(plan: Option<&work_plans::WorkPlanProjection>) -> Value {
    match plan {
        Some(plan) => json!({
            "domain": "activity",
            "plan_id": plan.id,
            "id": plan.id,
            "status": plan.status.clone(),
            "title": plan.title.clone(),
            "summary": plan.summary.clone(),
            "current_item": plan.current_item.clone(),
            "items": plan.items.clone(),
            "visibility": plan.visibility.clone(),
            "owner_role": plan.owner_role.clone(),
            "version": plan.version,
            "handoff_requested": plan.handoff_intent_path.is_some() || plan.handoff_task_id.is_some(),
            "handoff_intent_path": plan.handoff_intent_path.clone(),
            "handoff_task_id": plan.handoff_task_id.clone(),
            "updated_at": plan.updated_at,
        }),
        None => json!({
            "domain": "activity",
            "plan_id": Value::Null,
            "id": Value::Null,
            "status": "inactive",
            "title": Value::Null,
            "summary": Value::Null,
            "current_item": Value::Null,
            "items": [],
            "handoff_requested": false,
        }),
    }
}

// Den-executed server tools. Adding a new Den tool here and to
// `builtin_den_tool_descriptors` should not require an ACP adapter update when
// it uses existing stream/result shapes. Keep provider names semantic and
// provider-safe; accept legacy aliases only at routing boundaries.
pub const DEN_BEAR_GET_SELF: &str = "den.bear.get_self";
pub const DEN_USER_GET_CURRENT: &str = "den.user.get_current";
pub const DEN_BEAR_LIST_MEMBERS: &str = "den.bear.list_members";
pub const DEN_CAPABILITIES_LIST_SELF: &str = "den.capabilities.list_self";
pub const DEN_CHANNEL_GET_CONTEXT: &str = "den.channel.get_context";
pub const DEN_POLICY_GET_SELF: &str = "den.policy.get_self";
pub const DEN_CONVERSATION_SET_TITLE: &str = "den.conversation.set_title";
pub const DEN_CONVERSATION_SET_TITLE_PROVIDER: &str = "set_conversation_title";
pub const DEN_WEB_FETCH: &str = "den.web.fetch";
pub const DEN_WEB_FETCH_PROVIDER: &str = "web_fetch";
pub const DEN_WEB_FETCH_LEGACY_PROVIDER: &str = "den_web_fetch";
pub const DEN_WEB_SEARCH: &str = "den.web.search";
pub const DEN_WEB_SEARCH_PROVIDER: &str = "web_search";
pub const DEN_BEAR_ENVIRONMENT: &str = "den.bear.environment";
pub const DEN_BEAR_ENVIRONMENT_PROVIDER: &str = "bear_environment";
pub const DEN_SITUATION_GET: &str = "den.session.info";
pub const DEN_SITUATION_GET_PROVIDER: &str = "session_info";
pub const DEN_SITUATION_GET_LEGACY_PROVIDER: &str = "situation_get";
pub const DEN_MEMORY_WRITE_ENTRY: &str = "den.memory.write_entry";
pub const DEN_MEMORY_WRITE_ENTRY_PROVIDER: &str = "memory_write_entry";
pub const DEN_MEMORY_STATUS: &str = "den.memory.status";
pub const DEN_MEMORY_STATUS_PROVIDER: &str = "memory_status";
pub const DEN_MEMORY_TREE: &str = "den.memory.browse";
pub const DEN_MEMORY_TREE_PROVIDER: &str = "memory_browse";
pub const DEN_MEMORY_TREE_LEGACY_PROVIDER: &str = "memory_tree";
pub const DEN_MEMORY_READ: &str = "den.memory.read";
pub const DEN_MEMORY_READ_PROVIDER: &str = "memory_read";
pub const DEN_MEMORY_SEARCH: &str = "den.memory.search";
pub const DEN_MEMORY_SEARCH_PROVIDER: &str = "memory_search";
pub const DEN_MEMORY_ORIENT_WORK_SURFACE: &str = "den.memory.orient_work_surface";
pub const DEN_MEMORY_ORIENT_WORK_SURFACE_PROVIDER: &str = "memory_orient_work_surface";
pub const DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD: &str = "den.memory.create_work_surface_scaffold";
pub const DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD_PROVIDER: &str =
    "memory_create_work_surface_scaffold";
pub const DEN_MEMORY_REQUEST_REVIEW: &str = "den.memory.request_review";
pub const DEN_MEMORY_REQUEST_REVIEW_PROVIDER: &str = "memory_request_review";
pub const DEN_PROMPT_MEMORY_UPSERT: &str = "den.prompt_memory.upsert";
pub const DEN_PROMPT_MEMORY_UPSERT_PROVIDER: &str = "upsert_prompt_memory";
pub const DEN_PROMPT_MEMORY_LIST: &str = "den.prompt_memory.list";
pub const DEN_PROMPT_MEMORY_LIST_PROVIDER: &str = "list_prompt_memory";
pub const DEN_PROMPT_MEMORY_PATCH: &str = "den.prompt_memory.patch";
pub const DEN_PROMPT_MEMORY_PATCH_PROVIDER: &str = "patch_prompt_memory";
pub const DEN_MEMORY_LIST_PROPOSALS: &str = "den.memory.list_proposals";
pub const DEN_MEMORY_LIST_PROPOSALS_PROVIDER: &str = "memory_list_proposals";
pub const DEN_MEMORY_READ_PROPOSAL: &str = "den.memory.read_proposal";
pub const DEN_MEMORY_READ_PROPOSAL_PROVIDER: &str = "memory_read_proposal";
pub const DEN_MEMORY_RESOLVE_PROPOSAL: &str = "den.memory.resolve_proposal";
pub const DEN_MEMORY_RESOLVE_PROPOSAL_PROVIDER: &str = "memory_resolve_proposal";
pub const DEN_MEMORY_APPLY_CORE_UPDATE: &str = "den.memory.apply_core_update";
pub const DEN_MEMORY_APPLY_CORE_UPDATE_PROVIDER: &str = "memory_apply_core_update";
pub const DEN_SKILL_PROPOSE: &str = "den.skill.propose";
pub const DEN_SKILL_APPROVE_PROPOSAL: &str = "den.skill.approve_proposal";
pub const DEN_SKILL_REJECT_PROPOSAL: &str = "den.skill.reject_proposal";
pub const DEN_WORK_PLAN_LIST: &str = "den.work_plan.list";
pub const DEN_WORK_PLAN_LIST_PROVIDER: &str = "list_plans";
pub const DEN_WORK_PLAN_GET_STATUS: &str = "den.work_plan.get_status";
pub const DEN_WORK_PLAN_GET_STATUS_PROVIDER: &str = "get_plan_status";
pub const DEN_WORK_PLAN_UPDATE: &str = "den.work_plan.update";
pub const DEN_WORK_PLAN_UPDATE_PROVIDER: &str = "update_plan";
pub const DEN_WORK_PLAN_REQUEST_HANDOFF: &str = "den.work_plan.request_handoff";
pub const DEN_WORK_PLAN_REQUEST_HANDOFF_PROVIDER: &str = "request_work_handoff";
pub const DEN_PLAN_MODE_ENTER: &str = "den.plan_mode.enter";
pub const DEN_PLAN_MODE_ENTER_PROVIDER: &str = "enter_plan_mode";
pub const DEN_PLAN_MODE_STATUS: &str = "den.plan_mode.status";
pub const DEN_PLAN_MODE_STATUS_PROVIDER: &str = "get_plan_mode_status";
pub const DEN_PLAN_MODE_RECORD_APPROVAL: &str = "den.plan_mode.record_approval";
pub const DEN_PLAN_MODE_RECORD_APPROVAL_PROVIDER: &str = "record_plan_approval";
pub const DEN_PLAN_MODE_EXIT: &str = "den.plan_mode.exit";
pub const DEN_PLAN_MODE_EXIT_PROVIDER: &str = "exit_plan_mode";
pub const DEN_PLAN_MODE_CANCEL: &str = "den.plan_mode.cancel";
pub const DEN_PLAN_MODE_CANCEL_PROVIDER: &str = "cancel_plan_mode";
pub const DEN_TASK_WRITE_INTENT: &str = "den.task.write_intent";
pub const DEN_TASK_APPROVE_INTENT: &str = "den.task.approve_intent";
pub const DEN_TASK_REJECT_INTENT: &str = "den.task.reject_intent";
pub const DEN_CORE_WRITE_RESULT_SUMMARY: &str = "den.core.write_result_summary";
pub const DEN_OBSERVATION_WRITE: &str = "den.observation.write";
pub const DEN_RUN_WRITE_RESULT: &str = "den.run.write_result";

const ALL_ROLES: &[&str] = &["talk", "pair", "curate", "work", "watch"];
const WORK_PLAN_READ_ROLES: &[&str] = &["talk", "pair", "curate", "work"];
const WORK_PLAN_UPDATE_ROLES: &[&str] = &["talk", "pair", "work"];
const TALK_AND_PAIR_ROLES: &[&str] = &["talk", "pair"];
const PAIR_ROLES: &[&str] = &["pair"];
const PAIR_AND_CURATE_ROLES: &[&str] = &["pair", "curate"];
const CURATE_ROLES: &[&str] = &["curate"];
const WATCH_ROLES: &[&str] = &["watch"];
const WORK_ROLES: &[&str] = &["work"];

pub use crate::core::tools::descriptor::{
    builtin_den_tool_descriptors, builtin_den_tool_descriptors_for_role,
    builtin_den_tool_descriptor_for_provider_name, den_tool_display, provider_safe_tool_name,
    DenToolDescriptor,
};


pub fn provider_aliases_for_tool(name: &str) -> &'static [&'static str] {
    match name {
        DEN_WEB_FETCH => &[DEN_WEB_FETCH_LEGACY_PROVIDER],
        DEN_WEB_SEARCH => &["den_web_search"],
        DEN_CONVERSATION_SET_TITLE => &[
            "set_thread_title",
            "rename_conversation",
            "rename_thread",
            "conversation_rename",
        ],
        DEN_PLAN_MODE_RECORD_APPROVAL => &["approve_plan", "approve_current_plan"],
        DEN_BEAR_ENVIRONMENT => &["den_bear_environment"],
        DEN_SITUATION_GET => &[DEN_SITUATION_GET_LEGACY_PROVIDER, "den_situation_get"],
        DEN_MEMORY_WRITE_ENTRY => &["den_memory_write_entry"],
        DEN_MEMORY_STATUS => &["den_memory_status"],
        DEN_MEMORY_TREE => &[DEN_MEMORY_TREE_LEGACY_PROVIDER, "den_memory_tree"],
        DEN_MEMORY_READ => &["den_memory_read"],
        DEN_MEMORY_SEARCH => &["den_memory_search"],
        DEN_MEMORY_ORIENT_WORK_SURFACE => &["den_memory_orient_work_surface"],
        DEN_MEMORY_REQUEST_REVIEW => &["den_memory_request_review"],
        DEN_PROMPT_MEMORY_UPSERT => &["den_prompt_memory_upsert"],
        DEN_PROMPT_MEMORY_LIST => &["den_prompt_memory_list"],
        DEN_PROMPT_MEMORY_PATCH => &["den_prompt_memory_patch"],
        DEN_MEMORY_LIST_PROPOSALS => &["den_memory_list_proposals"],
        DEN_MEMORY_READ_PROPOSAL => &["den_memory_read_proposal"],
        DEN_MEMORY_RESOLVE_PROPOSAL => &["den_memory_resolve_proposal"],
        DEN_MEMORY_APPLY_CORE_UPDATE => &["den_memory_apply_core_update"],
        _ => &[],
    }
}

pub fn is_builtin_den_tool(name: &str) -> bool {
    matches!(
        name,
        DEN_BEAR_GET_SELF
            | DEN_USER_GET_CURRENT
            | DEN_BEAR_LIST_MEMBERS
            | DEN_CAPABILITIES_LIST_SELF
            | DEN_CHANNEL_GET_CONTEXT
            | DEN_POLICY_GET_SELF
            | DEN_BEAR_ENVIRONMENT
            | DEN_CONVERSATION_SET_TITLE
            | DEN_WEB_FETCH
            | DEN_WEB_SEARCH
            | DEN_SITUATION_GET
            | DEN_MEMORY_WRITE_ENTRY
            | DEN_MEMORY_STATUS
            | DEN_MEMORY_TREE
            | DEN_MEMORY_READ
            | DEN_MEMORY_SEARCH
            | DEN_MEMORY_ORIENT_WORK_SURFACE
            | DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD
            | DEN_MEMORY_REQUEST_REVIEW
            | DEN_PROMPT_MEMORY_UPSERT
            | DEN_PROMPT_MEMORY_LIST
            | DEN_PROMPT_MEMORY_PATCH
            | DEN_MEMORY_LIST_PROPOSALS
            | DEN_MEMORY_READ_PROPOSAL
            | DEN_MEMORY_RESOLVE_PROPOSAL
            | DEN_MEMORY_APPLY_CORE_UPDATE
            | DEN_SKILL_PROPOSE
            | DEN_SKILL_APPROVE_PROPOSAL
            | DEN_SKILL_REJECT_PROPOSAL
            | DEN_WORK_PLAN_LIST
            | DEN_WORK_PLAN_GET_STATUS
            | DEN_WORK_PLAN_UPDATE
            | DEN_WORK_PLAN_REQUEST_HANDOFF
            | DEN_PLAN_MODE_ENTER
            | DEN_PLAN_MODE_STATUS
            | DEN_PLAN_MODE_RECORD_APPROVAL
            | DEN_PLAN_MODE_EXIT
            | DEN_PLAN_MODE_CANCEL
            | DEN_TASK_WRITE_INTENT
            | DEN_TASK_APPROVE_INTENT
            | DEN_TASK_REJECT_INTENT
            | DEN_CORE_WRITE_RESULT_SUMMARY
            | DEN_OBSERVATION_WRITE
            | DEN_RUN_WRITE_RESULT
    )
}

pub use crate::core::tools::session::DenToolInvocationContext;
pub(crate) use crate::core::tools::support::validate_memory_write_entry_semantics;
pub(crate) use crate::core::tools::{
    environment::{bear_environment, fetch_acp_adapter_environment, session_info},
    memory_read::{memory_browse, memory_read, memory_search, memory_status, memory_status_value},
    memory_review::{
        apply_core_update, list_memory_proposals, read_memory_proposal,
        request_memory_review, resolve_memory_proposal, MemoryApplyCoreUpdateArguments,
        MemoryListProposalsArguments, MemoryReadProposalArguments,
        MemoryRequestReviewArguments, MemoryResolveProposalArguments,
    },
    memory_write::{merge_memory_entry_source_with_human, source_acp_session_id, write_memory_entry, MemoryWriteEntryArguments},
    plan_mode::{
        cancel_plan_mode, enter_plan_mode, exit_plan_mode, plan_mode_status,
        record_plan_approval, PlanModeCancelArguments, PlanModeEnterArguments,
        PlanModeExitArguments, PlanModeRecordApprovalArguments,
    },
    prompt_memory::{
        default_prompt_memory_state, empty_json_object, prompt_memory_list,
        prompt_memory_patch, prompt_memory_upsert, PromptMemoryListArguments,
        PromptMemoryPatchArguments, PromptMemoryUpsertArguments,
    },
    workflow::{
        empty_json_object as workflow_empty_json_object, get_work_plan_status,
        list_work_plans, update_work_plan, WorkPlanGetStatusArguments,
        WorkPlanListArguments, WorkPlanUpdateArguments,
    },
    work_surface::{
        build_work_surface_orientation_payload, collect_memory_tree_paths,
        create_work_surface_scaffold, infer_work_surface_hint, work_surface_anchor_paths,
        work_surface_candidate_slug, work_surface_entry_body, work_surface_index_file_body,
        work_surface_scaffold_requests,
    },
};
use crate::core::tools::session::authorize_tool_for_role;
use crate::core::tools::support::{
    assess_unlabeled_memory_misuse, clean_limited_strings, clean_optional,
    memory_read_scopes, memory_write_scopes, validate_bounded_text,
    validate_optional_object, validate_prompt_memory_scope,
};


#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DenToolChannelContext {
    pub family: Option<String>,
    pub client: Option<String>,
    pub protocol: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetConversationTitleArguments {
    title: String,
}

#[derive(Debug, Deserialize)]
struct MemoryCreateWorkSurfaceScaffoldArguments {
    work_surface_slug: String,
    work_surface_name: String,
    overview: String,
    #[serde(default)]
    glossary: Option<String>,
    #[serde(default)]
    current_understanding: Option<String>,
}

pub use crate::core::tools::session::invoke_den_tool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolSemanticWarning {
    pub code: &'static str,
    pub category: &'static str,
    pub message: String,
    pub confirmation_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolPreflight {
    Proceed,
    Warning(ToolSemanticWarning),
}

pub(crate) fn tool_warning_payload(tool_name: &str, warning: ToolSemanticWarning) -> Value {
    json!({
        "status": "warning",
        "tool_name": tool_name,
        "warning": {
            "code": warning.code,
            "category": warning.category,
            "message": warning.message,
            "confirmation_token": warning.confirmation_token,
        }
    })
}

pub(crate) fn prevalidate_tool_arguments(
    tool_name: &str,
    arguments: &Value,
    context: &DenToolInvocationContext,
) -> Result<ToolPreflight, CustomError> {
    match tool_name {
        DEN_MEMORY_WRITE_ENTRY => {
            let args: MemoryWriteEntryArguments = serde_json::from_value(arguments.clone())?;
            validate_memory_write_entry_semantics(&args, context)?;
            assess_unlabeled_memory_misuse(&args, context)
        }
        _ => Ok(ToolPreflight::Proceed),
    }
}

async fn patch_letta_conversation_summary(
    config: &Config,
    conversation_id: &str,
    summary: &str,
) -> Result<(), CustomError> {
    let base_url = config.letta_base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return Err(CustomError::System(
            "Letta is not configured (set LETTA_BASE_URL)".to_string(),
        ));
    }
    let url = format!("{base_url}/v1/conversations/{conversation_id}");
    let mut request = reqwest::Client::new()
        .patch(url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({ "summary": summary }));
    let key = config.letta_api_key.trim();
    if !key.is_empty() {
        request = request.header(AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = request
        .send()
        .await
        .map_err(|err| CustomError::System(format!("Letta patch conversation failed: {err}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(CustomError::System(format!(
            "Letta patch conversation HTTP {status}: {text}"
        )));
    }
    Ok(())
}

pub(crate) fn bear_environment_payload(
    context: &DenToolInvocationContext,
    config: &Config,
    role: BearAgentRole,
    current_user: Option<&user::User>,
    member_count: i64,
    memory_status: Value,
    adapter_runtime: Value,
) -> Value {
    let session_info = session_info_payload(
        context,
        role,
        current_user,
        member_count,
        memory_status.clone(),
    );
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
        "available_den_tools": builtin_den_tool_descriptors_for_role(role)
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
            "role": role.as_str(),
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
            "role": role.as_str(),
            "role_agent_id": context.role_agent_id,
            "member_count": member_count,
            "contract_label": match role {
                BearAgentRole::Pair => Value::String("Builder Bear".to_string()),
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
            "memfs_configured": !config.letta_memfs_service_url.trim().is_empty(),
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

pub(crate) fn session_info_payload(
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    current_user: Option<&user::User>,
    member_count: i64,
    memory_status: Value,
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
        "role": role.as_str(),
        "memory_surface": format!("{}/", role.as_str()),
        "space": match role {
            BearAgentRole::Pair => "Collaboration Space",
            BearAgentRole::Talk => "Conversation Space",
            BearAgentRole::Curate => "Curation Space",
            BearAgentRole::Work => "Execution Space",
            BearAgentRole::Watch => "Observation Space",
        },
    });
    let role_contract_label = match role {
        BearAgentRole::Pair => Some("Builder Bear"),
        _ => None,
    };
    json!({
        "role_contract_context": {
            "role": role.as_str(),
            "agent_id": context.role_agent_id,
            "contract_label": role_contract_label,
            "contract_source": if role_contract_label.is_some() { json!("system_prompt") } else { Value::Null },
            "contract_purpose": if role_contract_label.is_some() { json!("behavioral_style_and_role_guidance") } else { Value::Null },
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
        "role": {
            "name": role.as_str(),
            "agent_id": context.role_agent_id,
            "workplace": workplace,
        },
        "role_agent_id": context.role_agent_id,
        "human": {
            "user_id": context.user_id,
            "username": current_user.as_ref().map(|user| user.username.clone()).or_else(|| context.username.clone()),
            "display_name": current_user.as_ref().map(|user| user.display_name.clone()),
            "email_verified": current_user.as_ref().map(|user| user.email_verified.unwrap_or(false)),
            "membership_role": context.membership_role,
            "is_bear_admin": role_is_bear_admin(context.membership_role.as_deref()),
            "relationship": "authenticated ACP token owner; memory entries and logs should attribute work to this human"
        },
        "user": {
            "user_id": context.user_id,
            "username": current_user.as_ref().map(|user| user.username.clone()).or_else(|| context.username.clone()),
            "display_name": current_user.as_ref().map(|user| user.display_name.clone()),
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
            "available_tools": [
                DEN_MEMORY_WRITE_ENTRY_PROVIDER,
                DEN_MEMORY_STATUS_PROVIDER,
                DEN_MEMORY_TREE_PROVIDER,
                DEN_MEMORY_READ_PROVIDER,
                DEN_MEMORY_SEARCH_PROVIDER
            ],
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

fn prompt_memory_diagnostic_summary_for_bear_role(
    blocks: &[crate::core::prompt_memory_blocks::PromptMemoryBlock],
) -> Value {
    let mut active_by_scope = serde_json::Map::new();
    let mut active_by_type = serde_json::Map::new();
    for scope in [
        PromptMemoryBlockScope::BearWide,
        PromptMemoryBlockScope::RoleLocal,
        PromptMemoryBlockScope::WorkSurface,
        PromptMemoryBlockScope::Session,
    ] {
        let key = serde_json::to_string(&scope)
            .unwrap_or_else(|_| "unknown".to_string())
            .trim_matches('"')
            .to_string();
        let count = blocks
            .iter()
            .filter(|block| block.state == PromptMemoryBlockState::Active && block.scope == scope)
            .count();
        active_by_scope.insert(key, json!(count));
    }
    for block_type in [
        PromptMemoryBlockType::RoleGuidance,
        PromptMemoryBlockType::WorkSurfaceContext,
        PromptMemoryBlockType::SessionFocus,
        PromptMemoryBlockType::UserInstruction,
    ] {
        let key = serde_json::to_string(&block_type)
            .unwrap_or_else(|_| "unknown".to_string())
            .trim_matches('"')
            .to_string();
        let count = blocks
            .iter()
            .filter(|block| block.state == PromptMemoryBlockState::Active && block.block_type == block_type)
            .count();
        active_by_type.insert(key, json!(count));
    }
    let active_blocks = blocks
        .iter()
        .filter(|block| block.state == PromptMemoryBlockState::Active)
        .map(|block| {
            json!({
                "id": block.id,
                "scope": block.scope,
                "block_type": block.block_type,
                "title": block.title,
                "priority": block.priority,
                "work_surface": block.work_surface,
                "session_id": block.session_id,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "status": if active_blocks.is_empty() { "empty" } else { "ok" },
        "source": "prompt_memory_blocks",
        "active_count": active_blocks.len(),
        "active_by_scope": active_by_scope,
        "active_by_type": active_by_type,
        "active_blocks": active_blocks,
    })
}

async fn memory_orient_work_surface(
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
) -> Result<Value, CustomError> {
    let hint_payload = infer_work_surface_hint(context, role);
    let candidate_slug = work_surface_candidate_slug(context);
    let http = memfs_http_client("MemFS work-surface orientation client build failed")?;
    let tree = fetch_memfs_role_memory_tree(
        &http,
        &config.letta_memfs_service_url,
        context.bear_id,
        role.as_str(),
    )
    .await?;
    let Some(tree) = tree else {
        return Ok(json!({
            "ok": false,
            "configured": false,
            "message": "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)",
            "orientation": build_work_surface_orientation_payload(role, &hint_payload, &[], candidate_slug),
        }));
    };
    let mut files = Vec::new();
    collect_memory_tree_paths(&tree.files, &mut files);
    let orientation =
        build_work_surface_orientation_payload(role, &hint_payload, &files, candidate_slug);
    Ok(json!({
        "ok": tree.ok,
        "configured": true,
        "bear_id": context.bear_id,
        "role": role.as_str(),
        "canonical_tip": tree.canonical_tip,
        "orientation": orientation,
    }))
}

pub(crate) fn normalize_work_surface_slug(value: &str) -> Result<String, CustomError> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(CustomError::ValidationError(
            "work_surface_slug must not be empty".to_string(),
        ));
    }
    let normalized: String = trimmed
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let collapsed = normalized
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        return Err(CustomError::ValidationError(
            "work_surface_slug must include at least one letter or digit".to_string(),
        ));
    }
    if collapsed.len() > 80 {
        return Err(CustomError::ValidationError(
            "work_surface_slug must be 80 characters or fewer after normalization".to_string(),
        ));
    }
    Ok(collapsed)
}



fn memfs_http_client(error_prefix: &str) -> Result<reqwest::Client, CustomError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| CustomError::System(format!("{error_prefix}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use std::collections::HashSet;

    fn names_for_role(role: BearAgentRole) -> HashSet<&'static str> {
        builtin_den_tool_descriptors_for_role(role)
            .into_iter()
            .map(|descriptor| descriptor.name)
            .collect()
    }

    #[tokio::test]
    async fn prompt_memory_tools_round_trip_through_store() {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
        let pool = match PgPoolOptions::new().connect(&database_url).await {
            Ok(pool) => pool,
            Err(_) => return,
        };
        let migrate = sqlx::migrate!("./migrations").run(&pool).await;
        if migrate.is_err() {
            return;
        }
        let bear_id = Uuid::new_v4();
        let context = DenToolInvocationContext {
            bear_id,
            bear_slug: "test-bear".to_string(),
            role_agent_id: "agent-test".to_string(),
            agent_role: Some(BearAgentRole::Pair),
            user_id: 1,
            username: Some("tester".to_string()),
            membership_role: Some("owner".to_string()),
            conversation_id: "conv-test".to_string(),
            session_id: "sess-test".to_string(),
            acp_session_id: Some("sess-test".to_string()),
            conversation_selection: None,
            runtime_target: None,
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            runtime: None,
            context_budget: None,
            request_id: Some("req-test".to_string()),
            channel: DenToolChannelContext {
                family: Some("acp".to_string()),
                client: Some("zed".to_string()),
                protocol: Some("acp".to_string()),
            },
        };
        let upsert = prompt_memory_upsert(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({
                "block_id": format!("pm-{}", Uuid::new_v4()),
                "scope": "session",
                "block_type": "session_focus",
                "session_id": "sess-test",
                "title": "Current focus",
                "body": "Prioritize persisted prompt memory runtime wiring.",
                "priority": 7
            }),
        )
        .await
        .expect("upsert prompt memory block");
        assert_eq!(upsert["status"], "ok");
        let block_id = upsert["block_id"].as_str().unwrap().to_string();
        let listed = prompt_memory_list(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({}),
        )
        .await
        .expect("list prompt memory blocks");
        assert!(listed["blocks"].as_array().unwrap().iter().any(|b| b["id"] == block_id));
        let patched = prompt_memory_patch(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({
                "block_id": block_id,
                "state": "archived",
                "title": "Current focus (archived)",
                "body": "Archived prompt memory block.",
                "priority": 1
            }),
        )
        .await
        .expect("patch prompt memory block");
        assert_eq!(patched["state"], "archived");
        let listed_active = prompt_memory_list(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({}),
        )
        .await
        .expect("list active prompt memory blocks");
        assert!(!listed_active["blocks"].as_array().unwrap().iter().any(|b| b["id"] == patched["block_id"]));
        let listed_all = prompt_memory_list(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({"include_archived": true}),
        )
        .await
        .expect("list all prompt memory blocks");
        assert!(listed_all["blocks"].as_array().unwrap().iter().any(|b| b["id"] == patched["block_id"]));
    }


    #[tokio::test]
    async fn prompt_memory_runtime_selection_prefers_session_then_surface_then_role_then_bear() {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
        let pool = match PgPoolOptions::new().connect(&database_url).await {
            Ok(pool) => pool,
            Err(_) => return,
        };
        if sqlx::migrate!("./migrations").run(&pool).await.is_err() {
            return;
        }
        let bear_id = Uuid::new_v4();
        let role_slug = BearAgentRole::Pair.as_str();
        let session_id = format!("sess-{}", Uuid::new_v4());
        let work_surface = format!("ws-{}", Uuid::new_v4());
        let ids = [
            format!("pm-bear-{}", Uuid::new_v4()),
            format!("pm-role-{}", Uuid::new_v4()),
            format!("pm-surface-{}", Uuid::new_v4()),
            format!("pm-session-{}", Uuid::new_v4()),
        ];
        let writes = vec![
            PromptMemoryBlockWrite {
                block_id: ids[0].clone(),
                bear_id: Some(bear_id),
                role_slug: Some(role_slug.to_string()),
                scope: PromptMemoryBlockScope::BearWide,
                block_type: PromptMemoryBlockType::RoleGuidance,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: None,
                title: "Bear".to_string(),
                body: "Bear-wide guidance".to_string(),
                priority: 1,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: json!({}),
            },
            PromptMemoryBlockWrite {
                block_id: ids[1].clone(),
                bear_id: Some(bear_id),
                role_slug: Some(role_slug.to_string()),
                scope: PromptMemoryBlockScope::RoleLocal,
                block_type: PromptMemoryBlockType::RoleGuidance,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: None,
                title: "Role".to_string(),
                body: "Role guidance".to_string(),
                priority: 1,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: json!({}),
            },
            PromptMemoryBlockWrite {
                block_id: ids[2].clone(),
                bear_id: Some(bear_id),
                role_slug: Some(role_slug.to_string()),
                scope: PromptMemoryBlockScope::WorkSurface,
                block_type: PromptMemoryBlockType::WorkSurfaceContext,
                state: PromptMemoryBlockState::Active,
                work_surface: Some(work_surface.clone()),
                session_id: None,
                title: "Surface".to_string(),
                body: "Work-surface context".to_string(),
                priority: 1,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: json!({}),
            },
            PromptMemoryBlockWrite {
                block_id: ids[3].clone(),
                bear_id: Some(bear_id),
                role_slug: Some(role_slug.to_string()),
                scope: PromptMemoryBlockScope::Session,
                block_type: PromptMemoryBlockType::SessionFocus,
                state: PromptMemoryBlockState::Active,
                work_surface: None,
                session_id: Some(session_id.clone()),
                title: "Session".to_string(),
                body: "Session focus".to_string(),
                priority: 1,
                created_by_user_id: Some(1),
                supersedes_block_id: None,
                metadata: json!({}),
            },
        ];
        for write in &writes {
            upsert_prompt_memory_block(&pool, write).await.expect("seed prompt memory block");
        }
        let selection = crate::core::prompt_memory_block_store::select_prompt_memory_blocks_for_runtime(
            &pool,
            crate::core::prompt_memory_block_store::PromptMemoryBlockQuery {
                bear_id: Some(bear_id),
                role_slug,
                session_id: &session_id,
                work_surfaces: std::slice::from_ref(&work_surface),
            },
        )
        .await
        .expect("runtime selection");
        let compiled = crate::core::prompt_memory_blocks::compile_prompt_memory_blocks(
            &selection.blocks,
            crate::core::prompt_memory_blocks::PromptMemoryCompilationInput {
                role: role_slug,
                work_surfaces: std::slice::from_ref(&work_surface),
                session_id: &session_id,
                max_blocks: 4,
            },
        );
        let included_ids = compiled
            .included_blocks
            .iter()
            .map(|block| block.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(included_ids, vec![ids[3].clone(), ids[2].clone(), ids[1].clone(), ids[0].clone()]);
        assert_eq!(selection.diagnostic["matched_count"], 4);
    }

    #[tokio::test]
    async fn prompt_memory_upsert_archives_superseded_block() {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
        let pool = match PgPoolOptions::new().connect(&database_url).await {
            Ok(pool) => pool,
            Err(_) => return,
        };
        if sqlx::migrate!("./migrations").run(&pool).await.is_err() {
            return;
        }
        let bear_id = Uuid::new_v4();
        let context = DenToolInvocationContext {
            bear_id,
            bear_slug: "test-bear".to_string(),
            role_agent_id: "agent-test".to_string(),
            agent_role: Some(BearAgentRole::Pair),
            user_id: 1,
            username: Some("tester".to_string()),
            membership_role: Some("owner".to_string()),
            conversation_id: "conv-test".to_string(),
            session_id: "sess-test".to_string(),
            acp_session_id: Some("sess-test".to_string()),
            conversation_selection: None,
            runtime_target: None,
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            runtime: None,
            context_budget: None,
            request_id: Some("req-test".to_string()),
            channel: DenToolChannelContext {
                family: Some("acp".to_string()),
                client: Some("zed".to_string()),
                protocol: Some("acp".to_string()),
            },
        };
        let original_block_id = format!("pm-original-{}", Uuid::new_v4());
        prompt_memory_upsert(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({
                "block_id": original_block_id,
                "scope": "session",
                "block_type": "session_focus",
                "session_id": "sess-test",
                "title": "Original focus",
                "body": "Original body",
                "priority": 3
            }),
        )
        .await
        .expect("upsert original block");
        let replacement = prompt_memory_upsert(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({
                "block_id": format!("pm-replacement-{}", Uuid::new_v4()),
                "scope": "session",
                "block_type": "session_focus",
                "session_id": "sess-test",
                "title": "Replacement focus",
                "body": "Replacement body",
                "priority": 5,
                "supersedes_block_id": original_block_id
            }),
        )
        .await
        .expect("upsert replacement block");
        assert_eq!(replacement["superseded_archived_count"], 1);
        let listed_all = prompt_memory_list(&pool, &context, BearAgentRole::Pair, json!({"include_archived": true}))
            .await
            .expect("list prompt memory blocks");
        let original = listed_all["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|block| block["id"] == replacement["supersedes_block_id"])
            .expect("original block should still be listed when archived included");
        assert_eq!(original["state"], "archived");
    }



    #[tokio::test]
    async fn prompt_memory_upsert_archives_conflicting_active_block_in_same_scope() {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
        let pool = match PgPoolOptions::new().connect(&database_url).await {
            Ok(pool) => pool,
            Err(_) => return,
        };
        if sqlx::migrate!("./migrations").run(&pool).await.is_err() {
            return;
        }
        let bear_id = Uuid::new_v4();
        let context = DenToolInvocationContext {
            bear_id,
            bear_slug: "test-bear".to_string(),
            role_agent_id: "agent-test".to_string(),
            agent_role: Some(BearAgentRole::Pair),
            user_id: 1,
            username: Some("tester".to_string()),
            membership_role: Some("owner".to_string()),
            conversation_id: "conv-test".to_string(),
            session_id: "sess-test".to_string(),
            acp_session_id: Some("sess-test".to_string()),
            conversation_selection: None,
            runtime_target: None,
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            runtime: None,
            context_budget: None,
            request_id: Some("req-test".to_string()),
            channel: DenToolChannelContext {
                family: Some("acp".to_string()),
                client: Some("zed".to_string()),
                protocol: Some("acp".to_string()),
            },
        };
        prompt_memory_upsert(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({
                "block_id": format!("pm-conflict-a-{}", Uuid::new_v4()),
                "scope": "session",
                "block_type": "session_focus",
                "session_id": "sess-test",
                "title": "First focus",
                "body": "First body",
                "priority": 3
            }),
        )
        .await
        .expect("upsert first block");
        let replacement = prompt_memory_upsert(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({
                "block_id": format!("pm-conflict-b-{}", Uuid::new_v4()),
                "scope": "session",
                "block_type": "session_focus",
                "session_id": "sess-test",
                "title": "Second focus",
                "body": "Second body",
                "priority": 8
            }),
        )
        .await
        .expect("upsert second block");
        assert_eq!(replacement["conflicting_archived_count"], 1);
        let active = prompt_memory_list(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({"scope": "session", "block_type": "session_focus", "session_id": "sess-test"}),
        )
        .await
        .expect("list active session prompt memory blocks");
        assert_eq!(active["count"], 1);
    }

    #[tokio::test]
    async fn memory_status_includes_prompt_memory_diagnostic_summary() {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1/postgres".to_string());
        let pool = match PgPoolOptions::new().connect(&database_url).await {
            Ok(pool) => pool,
            Err(_) => return,
        };
        if sqlx::migrate!("./migrations").run(&pool).await.is_err() {
            return;
        }
        let bear_id = Uuid::new_v4();
        let context = DenToolInvocationContext {
            bear_id,
            bear_slug: "test-bear".to_string(),
            role_agent_id: "agent-test".to_string(),
            agent_role: Some(BearAgentRole::Pair),
            user_id: 1,
            username: Some("tester".to_string()),
            membership_role: Some("owner".to_string()),
            conversation_id: "conv-test".to_string(),
            session_id: "sess-test".to_string(),
            acp_session_id: Some("sess-test".to_string()),
            conversation_selection: None,
            runtime_target: None,
            workspace_roots: vec!["/workspace".to_string()],
            session_policy: None,
            activity: None,
            runtime: None,
            context_budget: None,
            request_id: Some("req-test".to_string()),
            channel: DenToolChannelContext {
                family: Some("acp".to_string()),
                client: Some("zed".to_string()),
                protocol: Some("acp".to_string()),
            },
        };
        prompt_memory_upsert(
            &pool,
            &context,
            BearAgentRole::Pair,
            json!({
                "block_id": format!("pm-status-{}", Uuid::new_v4()),
                "scope": "session",
                "block_type": "session_focus",
                "session_id": "sess-test",
                "title": "Status focus",
                "body": "Status body",
                "priority": 4
            }),
        )
        .await
        .expect("upsert status block");
        let config = Config::test_stub();
        let status = memory_status_value(&config, &context, BearAgentRole::Pair, &pool)
            .await
            .expect("memory status value");
        assert_eq!(status["prompt_memory_diagnostic"]["source"], "prompt_memory_blocks");
        assert_eq!(status["prompt_memory_diagnostic"]["active_by_scope"]["session"], 1);
    }

    #[test]
    fn provider_names_are_safe_and_unique() {
        let descriptors = builtin_den_tool_descriptors();
        let mut provider_names = HashSet::new();
        for descriptor in descriptors {
            assert!(
                descriptor
                    .provider_name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "provider name must be Letta/provider-safe: {}",
                descriptor.provider_name
            );
            assert!(!descriptor.provider_name.contains('.'));
            assert!(!descriptor.provider_name.contains('/'));
            assert!(
                provider_names.insert(descriptor.provider_name.clone()),
                "duplicate provider name: {}",
                descriptor.provider_name
            );
        }
    }

    #[test]
    fn canonical_dotted_names_map_to_provider_safe_aliases() {
        let descriptors = builtin_den_tool_descriptors();
        let task = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_TASK_WRITE_INTENT)
            .expect("task intent descriptor exists");
        assert_eq!(task.provider_name, "den_task_write_intent");

        let skill = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_SKILL_PROPOSE)
            .expect("skill proposal descriptor exists");
        assert_eq!(skill.provider_name, "den_skill_propose");

        let conversation_title = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_CONVERSATION_SET_TITLE)
            .expect("conversation title descriptor exists");
        assert_eq!(
            conversation_title.provider_name,
            DEN_CONVERSATION_SET_TITLE_PROVIDER
        );

        let web_fetch = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_WEB_FETCH)
            .expect("web fetch descriptor exists");
        assert_eq!(web_fetch.provider_name, DEN_WEB_FETCH_PROVIDER);

        let web_search = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_WEB_SEARCH)
            .expect("web search descriptor exists");
        assert_eq!(web_search.provider_name, DEN_WEB_SEARCH_PROVIDER);

        let bear_environment = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_BEAR_ENVIRONMENT)
            .expect("bear environment descriptor exists");
        assert_eq!(
            bear_environment.provider_name,
            DEN_BEAR_ENVIRONMENT_PROVIDER
        );

        let situation = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_SITUATION_GET)
            .expect("situation descriptor exists");
        assert_eq!(situation.provider_name, DEN_SITUATION_GET_PROVIDER);
        assert_eq!(situation.provider_name, "session_info");
        assert_ne!(situation.provider_name, "situation_get");
        assert_ne!(situation.provider_name, "den_situation_get");

        let memory_browse = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_MEMORY_TREE)
            .expect("memory browse descriptor exists");
        assert_eq!(memory_browse.provider_name, DEN_MEMORY_TREE_PROVIDER);
        assert_eq!(memory_browse.provider_name, "memory_browse");
        assert_ne!(memory_browse.provider_name, "memory_tree");

        let memory = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_MEMORY_WRITE_ENTRY)
            .expect("memory write descriptor exists");
        assert_eq!(memory.provider_name, DEN_MEMORY_WRITE_ENTRY_PROVIDER);

        let update_plan = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_WORK_PLAN_UPDATE)
            .expect("work plan update descriptor exists");
        assert_eq!(update_plan.provider_name, DEN_WORK_PLAN_UPDATE_PROVIDER);
        assert_eq!(update_plan.provider_name, "update_plan");

        let enter_plan_mode = descriptors
            .iter()
            .find(|descriptor| descriptor.name == DEN_PLAN_MODE_ENTER)
            .expect("enter plan mode descriptor exists");
        assert_eq!(enter_plan_mode.provider_name, DEN_PLAN_MODE_ENTER_PROVIDER);
        assert_eq!(enter_plan_mode.provider_name, "enter_plan_mode");
    }

    #[test]
    fn den_server_tools_advertise_semantic_aliases_not_legacy_den_prefixes() {
        let provider_names = builtin_den_tool_descriptors_for_role(BearAgentRole::Pair)
            .into_iter()
            .map(|descriptor| descriptor.provider_name)
            .collect::<HashSet<_>>();
        assert!(provider_names.contains("session_info"));
        assert!(provider_names.contains("bear_environment"));
        assert!(provider_names.contains("set_conversation_title"));
        assert!(provider_names.contains("web_search"));
        assert!(provider_names.contains("memory_browse"));
        assert!(provider_names.contains("memory_read"));
        assert!(provider_names.contains("update_plan"));
        assert!(provider_names.contains("enter_plan_mode"));
        assert!(provider_names.contains("record_plan_approval"));
        assert!(provider_names.contains("exit_plan_mode"));
        assert!(provider_names.contains("cancel_plan_mode"));
        assert!(!provider_names.contains("situation_get"));
        assert!(!provider_names.contains("memory_tree"));
        assert!(!provider_names.contains("den_situation_get"));
        assert!(!provider_names.contains("den_web_search"));
        assert!(!provider_names.contains("den_memory_read"));
        assert!(!provider_names.contains("den_work_plan_update"));
        assert!(!provider_names.contains("den_plan_mode_enter"));
    }

    #[test]
    fn bear_environment_payload_exposes_baseline_sections() {
        let context = DenToolInvocationContext {
            bear_id: Uuid::nil(),
            bear_slug: "meta".to_string(),
            role_agent_id: "agent-123".to_string(),
            agent_role: Some(BearAgentRole::Pair),
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
            BearAgentRole::Pair,
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

    #[test]
    fn privileged_descriptors_are_role_scoped() {
        let talk = names_for_role(BearAgentRole::Talk);
        assert!(talk.contains(DEN_TASK_WRITE_INTENT));
        assert!(talk.contains(DEN_SKILL_PROPOSE));
        assert!(!talk.contains(DEN_OBSERVATION_WRITE));
        assert!(!talk.contains(DEN_RUN_WRITE_RESULT));

        let pair = names_for_role(BearAgentRole::Pair);
        assert!(pair.contains(DEN_TASK_WRITE_INTENT));
        assert!(pair.contains(DEN_WORK_PLAN_UPDATE));
        assert!(pair.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
        assert!(pair.contains(DEN_SKILL_PROPOSE));
        assert!(!pair.contains(DEN_OBSERVATION_WRITE));
        assert!(!pair.contains(DEN_RUN_WRITE_RESULT));

        let curate = names_for_role(BearAgentRole::Curate);
        assert!(curate.contains(DEN_TASK_APPROVE_INTENT));
        assert!(curate.contains(DEN_TASK_REJECT_INTENT));
        assert!(curate.contains(DEN_CORE_WRITE_RESULT_SUMMARY));
        assert!(curate.contains(DEN_SKILL_APPROVE_PROPOSAL));
        assert!(curate.contains(DEN_SKILL_REJECT_PROPOSAL));
        assert!(curate.contains(DEN_SKILL_PROPOSE));
        assert!(!curate.contains(DEN_TASK_WRITE_INTENT));
        assert!(!curate.contains(DEN_OBSERVATION_WRITE));
        assert!(!curate.contains(DEN_RUN_WRITE_RESULT));

        let watch = names_for_role(BearAgentRole::Watch);
        assert!(watch.contains(DEN_OBSERVATION_WRITE));
        assert!(watch.contains(DEN_SKILL_PROPOSE));
        assert!(!watch.contains(DEN_WORK_PLAN_LIST));
        assert!(!watch.contains(DEN_WORK_PLAN_UPDATE));
        assert!(!watch.contains(DEN_TASK_WRITE_INTENT));
        assert!(!watch.contains(DEN_RUN_WRITE_RESULT));

        let work = names_for_role(BearAgentRole::Work);
        assert!(work.contains(DEN_RUN_WRITE_RESULT));
        assert!(work.contains(DEN_WORK_PLAN_LIST));
        assert!(work.contains(DEN_WORK_PLAN_UPDATE));
        assert!(!work.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
        assert!(work.contains(DEN_SKILL_PROPOSE));
        assert!(!work.contains(DEN_TASK_WRITE_INTENT));
        assert!(!work.contains(DEN_OBSERVATION_WRITE));
    }

    #[test]
    fn all_descriptors_are_known_tools() {
        for descriptor in builtin_den_tool_descriptors() {
            assert!(
                is_builtin_den_tool(descriptor.name),
                "unknown descriptor name: {}",
                descriptor.name
            );
        }
    }

    #[test]
    fn pair_has_web_memory_and_activity_tools() {
        let pair = names_for_role(BearAgentRole::Pair);
        assert!(pair.contains(DEN_CONVERSATION_SET_TITLE));
        assert!(pair.contains(DEN_WEB_FETCH));
        assert!(pair.contains(DEN_WEB_SEARCH));
        assert!(pair.contains(DEN_BEAR_ENVIRONMENT));
        assert!(pair.contains(DEN_SITUATION_GET));
        assert!(pair.contains(DEN_MEMORY_WRITE_ENTRY));
        assert!(pair.contains(DEN_MEMORY_STATUS));
        assert!(pair.contains(DEN_MEMORY_TREE));
        assert!(pair.contains(DEN_MEMORY_READ));
        assert!(pair.contains(DEN_MEMORY_SEARCH));
        assert!(pair.contains(DEN_WORK_PLAN_LIST));
        assert!(pair.contains(DEN_WORK_PLAN_GET_STATUS));
        assert!(pair.contains(DEN_WORK_PLAN_UPDATE));
        assert!(pair.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
        assert!(pair.contains(DEN_PLAN_MODE_ENTER));
        assert!(pair.contains(DEN_PLAN_MODE_STATUS));
        assert!(pair.contains(DEN_PLAN_MODE_RECORD_APPROVAL));
        assert!(pair.contains(DEN_PLAN_MODE_EXIT));
        assert!(pair.contains(DEN_PLAN_MODE_CANCEL));

        let talk = names_for_role(BearAgentRole::Talk);
        assert!(talk.contains(DEN_CONVERSATION_SET_TITLE));
        assert!(!talk.contains(DEN_WEB_FETCH));
        assert!(!talk.contains(DEN_WEB_SEARCH));
        assert!(!talk.contains(DEN_MEMORY_WRITE_ENTRY));
    }

    #[tokio::test]
    async fn web_search_reports_missing_provider_config() {
        let config = Config::test_stub();
        let err = crate::core::tools::web::web_search_inner(
            None,
            &config,
            None,
            json!({ "query": "rust docs" }),
        )
        .await
        .expect_err("missing provider should fail clearly");
        assert!(err.to_string().contains("DEN_SEARCH_PROVIDER"));
    }

    #[test]
    fn role_authorization_rejects_disallowed_tools() {
        assert!(authorize_tool_for_role(DEN_TASK_WRITE_INTENT, BearAgentRole::Talk).is_ok());
        assert!(authorize_tool_for_role(DEN_TASK_WRITE_INTENT, BearAgentRole::Watch).is_err());
        assert!(authorize_tool_for_role(DEN_RUN_WRITE_RESULT, BearAgentRole::Work).is_ok());
        assert!(authorize_tool_for_role(DEN_RUN_WRITE_RESULT, BearAgentRole::Talk).is_err());
        assert!(authorize_tool_for_role(DEN_TASK_APPROVE_INTENT, BearAgentRole::Curate).is_ok());
        assert!(authorize_tool_for_role(DEN_TASK_APPROVE_INTENT, BearAgentRole::Pair).is_err());
        assert!(authorize_tool_for_role(DEN_SKILL_APPROVE_PROPOSAL, BearAgentRole::Curate).is_ok());
        assert!(authorize_tool_for_role(DEN_SKILL_APPROVE_PROPOSAL, BearAgentRole::Work).is_err());
        assert!(authorize_tool_for_role(DEN_WORK_PLAN_UPDATE, BearAgentRole::Pair).is_ok());
        assert!(authorize_tool_for_role(DEN_WORK_PLAN_UPDATE, BearAgentRole::Watch).is_err());
        assert!(
            authorize_tool_for_role(DEN_WORK_PLAN_REQUEST_HANDOFF, BearAgentRole::Work).is_err()
        );
    }
}
