use crate::core::den_tools::{
    DEN_BEAR_ENVIRONMENT, DEN_CONVERSATION_SET_TITLE, DEN_MEMORY_APPLY_CORE_UPDATE,
    DEN_MEMORY_LIST_PROPOSALS, DEN_MEMORY_ORIENT_WORK_SURFACE, DEN_MEMORY_READ,
    DEN_MEMORY_READ_PROPOSAL, DEN_MEMORY_REQUEST_REVIEW, DEN_MEMORY_RESOLVE_PROPOSAL,
    DEN_MEMORY_SEARCH, DEN_MEMORY_STATUS, DEN_MEMORY_TREE, DEN_MEMORY_TREE_LEGACY_PROVIDER,
    DEN_MEMORY_WRITE_ENTRY, DEN_PLAN_MODE_RECORD_APPROVAL, DEN_PROMPT_MEMORY_LIST,
    DEN_PROMPT_MEMORY_PATCH, DEN_PROMPT_MEMORY_UPSERT, DEN_SITUATION_GET,
    DEN_SITUATION_GET_LEGACY_PROVIDER, DEN_WEB_FETCH, DEN_WEB_FETCH_LEGACY_PROVIDER,
    DEN_WEB_SEARCH,
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
        crate::core::den_tools::DEN_BEAR_GET_SELF
            | crate::core::den_tools::DEN_USER_GET_CURRENT
            | crate::core::den_tools::DEN_BEAR_LIST_MEMBERS
            | crate::core::den_tools::DEN_CAPABILITIES_LIST_SELF
            | crate::core::den_tools::DEN_CHANNEL_GET_CONTEXT
            | crate::core::den_tools::DEN_POLICY_GET_SELF
            | crate::core::den_tools::DEN_BEAR_ENVIRONMENT
            | crate::core::den_tools::DEN_CONVERSATION_SET_TITLE
            | crate::core::den_tools::DEN_WEB_FETCH
            | crate::core::den_tools::DEN_WEB_SEARCH
            | crate::core::den_tools::DEN_SITUATION_GET
            | crate::core::den_tools::DEN_MEMORY_WRITE_ENTRY
            | crate::core::den_tools::DEN_MEMORY_STATUS
            | crate::core::den_tools::DEN_MEMORY_TREE
            | crate::core::den_tools::DEN_MEMORY_READ
            | crate::core::den_tools::DEN_MEMORY_SEARCH
            | crate::core::den_tools::DEN_MEMORY_ORIENT_WORK_SURFACE
            | crate::core::den_tools::DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD
            | crate::core::den_tools::DEN_MEMORY_REQUEST_REVIEW
            | crate::core::den_tools::DEN_PROMPT_MEMORY_UPSERT
            | crate::core::den_tools::DEN_PROMPT_MEMORY_LIST
            | crate::core::den_tools::DEN_PROMPT_MEMORY_PATCH
            | crate::core::den_tools::DEN_MEMORY_LIST_PROPOSALS
            | crate::core::den_tools::DEN_MEMORY_READ_PROPOSAL
            | crate::core::den_tools::DEN_MEMORY_RESOLVE_PROPOSAL
            | crate::core::den_tools::DEN_MEMORY_APPLY_CORE_UPDATE
            | crate::core::den_tools::DEN_SKILL_PROPOSE
            | crate::core::den_tools::DEN_SKILL_APPROVE_PROPOSAL
            | crate::core::den_tools::DEN_SKILL_REJECT_PROPOSAL
            | crate::core::den_tools::DEN_WORK_PLAN_LIST
            | crate::core::den_tools::DEN_WORK_PLAN_GET_STATUS
            | crate::core::den_tools::DEN_WORK_PLAN_UPDATE
            | crate::core::den_tools::DEN_WORK_PLAN_REQUEST_HANDOFF
            | crate::core::den_tools::DEN_PLAN_MODE_ENTER
            | crate::core::den_tools::DEN_PLAN_MODE_STATUS
            | crate::core::den_tools::DEN_PLAN_MODE_RECORD_APPROVAL
            | crate::core::den_tools::DEN_PLAN_MODE_EXIT
            | crate::core::den_tools::DEN_PLAN_MODE_CANCEL
            | crate::core::den_tools::DEN_TASK_WRITE_INTENT
            | crate::core::den_tools::DEN_TASK_APPROVE_INTENT
            | crate::core::den_tools::DEN_TASK_REJECT_INTENT
            | crate::core::den_tools::DEN_CORE_WRITE_RESULT_SUMMARY
            | crate::core::den_tools::DEN_OBSERVATION_WRITE
            | crate::core::den_tools::DEN_RUN_WRITE_RESULT
    )
}
