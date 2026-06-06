
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

pub use crate::core::tools::aliases::{is_builtin_den_tool, provider_aliases_for_tool};
pub use crate::core::tools::descriptor::{
    builtin_den_tool_descriptors, builtin_den_tool_descriptors_for_role,
    builtin_den_tool_descriptor_for_provider_name, den_tool_display, provider_safe_tool_name,
    DenToolDescriptor,
};


pub use crate::core::tools::{
    arguments::{
        DenToolChannelContext, MemoryCreateWorkSurfaceScaffoldArguments,
        SetConversationTitleArguments,
    },
    session::DenToolInvocationContext,
};
#[allow(unused_imports)]
pub(crate) use crate::core::tools::support::validate_memory_write_entry_semantics;
#[allow(unused_imports)]
pub(crate) use crate::core::tools::{
    activity_payloads::{activity_payload, no_active_workplan_payload, plan_mode_workplan_payload},
    environment::{bear_environment, fetch_acp_adapter_environment, session_info},
    letta::patch_letta_conversation_summary,
    memory_read::{memory_browse, memory_read, memory_search, memory_status, memory_status_value},
    payloads::{bear_environment_payload, session_info_payload},
    preflight::{prevalidate_tool_arguments, tool_warning_payload, ToolPreflight, ToolSemanticWarning},
    prompt_memory_diagnostics::prompt_memory_diagnostic_summary_for_bear_role,
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
        create_work_surface_scaffold, infer_work_surface_hint, normalize_work_surface_slug,
        work_surface_anchor_paths, work_surface_candidate_slug, work_surface_entry_body,
        work_surface_index_file_body, work_surface_scaffold_requests,
    },
};
pub use crate::core::tools::session::invoke_den_tool;

