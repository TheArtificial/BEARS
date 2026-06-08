use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const ALL_ROLES: &[&str] = &["chat", "pair", "curate", "work", "watch"];
const WORK_PLAN_READ_ROLES: &[&str] = &["chat", "pair", "curate", "work"];
const WORK_PLAN_UPDATE_ROLES: &[&str] = &["chat", "pair", "work"];
const CHAT_AND_PAIR_ROLES: &[&str] = &["chat", "pair"];
const PAIR_ROLES: &[&str] = &["pair"];
const PAIR_AND_CURATE_ROLES: &[&str] = &["pair", "curate"];
const CURATE_ROLES: &[&str] = &["curate"];
const WATCH_ROLES: &[&str] = &["watch"];
const WORK_ROLES: &[&str] = &["work"];

use crate::core::{
    acp_tools::AcpToolDisplayDescriptor,
    bears::BearAgentRole,
    tool_descriptor_guidance::{
        render_tool_descriptor_guidance, ToolDescriptorGuidance, ToolOrientationPolicy,
        ToolScopeKind, ToolSideEffectKind,
    },
    tools::constants::{
        DEN_BEAR_ENVIRONMENT, DEN_BEAR_ENVIRONMENT_PROVIDER,
        DEN_BEAR_GET_SELF, DEN_BEAR_LIST_MEMBERS, DEN_CAPABILITIES_LIST_SELF,
        DEN_CHANNEL_GET_CONTEXT, DEN_CONVERSATION_SET_TITLE,
        DEN_CONVERSATION_SET_TITLE_PROVIDER, DEN_CORE_WRITE_RESULT_SUMMARY,
        DEN_MEMORY_APPLY_CORE_UPDATE, DEN_MEMORY_APPLY_CORE_UPDATE_PROVIDER,
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD,
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD_PROVIDER, DEN_MEMORY_LIST_PROPOSALS,
        DEN_MEMORY_LIST_PROPOSALS_PROVIDER, DEN_MEMORY_ORIENT_WORK_SURFACE,
        DEN_MEMORY_ORIENT_WORK_SURFACE_PROVIDER, DEN_MEMORY_READ,
        DEN_MEMORY_READ_PROPOSAL, DEN_MEMORY_READ_PROPOSAL_PROVIDER, DEN_MEMORY_READ_PROVIDER,
        DEN_MEMORY_REQUEST_REVIEW, DEN_MEMORY_REQUEST_REVIEW_PROVIDER,
        DEN_MEMORY_RESOLVE_PROPOSAL, DEN_MEMORY_RESOLVE_PROPOSAL_PROVIDER,
        DEN_MEMORY_SEARCH, DEN_MEMORY_SEARCH_PROVIDER, DEN_MEMORY_STATUS,
        DEN_MEMORY_STATUS_PROVIDER, DEN_MEMORY_TREE, DEN_MEMORY_TREE_LEGACY_PROVIDER,
        DEN_MEMORY_TREE_PROVIDER, DEN_MEMORY_WRITE_ENTRY, DEN_MEMORY_WRITE_ENTRY_PROVIDER,
        DEN_OBSERVATION_WRITE, DEN_PLAN_MODE_CANCEL, DEN_PLAN_MODE_CANCEL_PROVIDER,
        DEN_PLAN_MODE_ENTER, DEN_PLAN_MODE_ENTER_PROVIDER, DEN_PLAN_MODE_EXIT,
        DEN_PLAN_MODE_EXIT_PROVIDER, DEN_PLAN_MODE_RECORD_APPROVAL,
        DEN_PLAN_MODE_RECORD_APPROVAL_PROVIDER, DEN_PLAN_MODE_STATUS,
        DEN_PLAN_MODE_STATUS_PROVIDER, DEN_POLICY_GET_SELF, DEN_PROMPT_MEMORY_LIST,
        DEN_PROMPT_MEMORY_LIST_PROVIDER, DEN_PROMPT_MEMORY_PATCH,
        DEN_PROMPT_MEMORY_PATCH_PROVIDER, DEN_PROMPT_MEMORY_UPSERT,
        DEN_PROMPT_MEMORY_UPSERT_PROVIDER, DEN_RUN_WRITE_RESULT, DEN_SITUATION_GET,
        DEN_SITUATION_GET_LEGACY_PROVIDER, DEN_SITUATION_GET_PROVIDER,
        DEN_SKILL_APPROVE_PROPOSAL, DEN_SKILL_PROPOSE, DEN_SKILL_REJECT_PROPOSAL,
        DEN_TASK_APPROVE_INTENT, DEN_TASK_REJECT_INTENT, DEN_TASK_WRITE_INTENT,
        DEN_USER_GET_CURRENT, DEN_WEB_FETCH, DEN_WEB_FETCH_LEGACY_PROVIDER,
        DEN_WEB_FETCH_PROVIDER, DEN_WEB_SEARCH, DEN_WEB_SEARCH_PROVIDER,
        DEN_WORK_PLAN_GET_STATUS, DEN_WORK_PLAN_GET_STATUS_PROVIDER, DEN_WORK_PLAN_LIST,
        DEN_WORK_PLAN_LIST_PROVIDER, DEN_WORK_PLAN_REQUEST_HANDOFF,
        DEN_WORK_PLAN_REQUEST_HANDOFF_PROVIDER, DEN_WORK_PLAN_UPDATE,
        DEN_WORK_PLAN_UPDATE_PROVIDER,
    },
};

#[derive(Debug, Clone, Serialize)]
pub struct DenToolDescriptor {
    pub name: &'static str,
    pub provider_name: String,
    pub provider_aliases: &'static [&'static str],
    pub label: &'static str,
    pub description: &'static str,
    pub kind: &'static str,
    pub provider: &'static str,
    pub execution_target: &'static str,
    pub scope: &'static str,
    pub domain: &'static str,
    pub content_class: Option<&'static str>,
    pub availability: &'static str,
    pub permissions: &'static [&'static str],
    pub allowed_roles: &'static [&'static str],
    pub approval_policy: &'static str,
    pub display: serde_json::Value,
    pub input_schema: Value,
}

pub fn provider_safe_tool_name(name: &str) -> String {
    match name {
        DEN_CONVERSATION_SET_TITLE => return DEN_CONVERSATION_SET_TITLE_PROVIDER.to_string(),
        DEN_WEB_FETCH => return DEN_WEB_FETCH_PROVIDER.to_string(),
        DEN_WEB_SEARCH => return DEN_WEB_SEARCH_PROVIDER.to_string(),
        DEN_BEAR_ENVIRONMENT => return DEN_BEAR_ENVIRONMENT_PROVIDER.to_string(),
        DEN_SITUATION_GET => return DEN_SITUATION_GET_PROVIDER.to_string(),
        DEN_MEMORY_WRITE_ENTRY => return DEN_MEMORY_WRITE_ENTRY_PROVIDER.to_string(),
        DEN_MEMORY_STATUS => return DEN_MEMORY_STATUS_PROVIDER.to_string(),
        DEN_MEMORY_TREE => return DEN_MEMORY_TREE_PROVIDER.to_string(),
        DEN_MEMORY_READ => return DEN_MEMORY_READ_PROVIDER.to_string(),
        DEN_MEMORY_SEARCH => return DEN_MEMORY_SEARCH_PROVIDER.to_string(),
        DEN_MEMORY_ORIENT_WORK_SURFACE => {
            return DEN_MEMORY_ORIENT_WORK_SURFACE_PROVIDER.to_string()
        }
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD => {
            return DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD_PROVIDER.to_string()
        }
        DEN_MEMORY_REQUEST_REVIEW => return DEN_MEMORY_REQUEST_REVIEW_PROVIDER.to_string(),
        DEN_PROMPT_MEMORY_UPSERT => return DEN_PROMPT_MEMORY_UPSERT_PROVIDER.to_string(),
        DEN_PROMPT_MEMORY_LIST => return DEN_PROMPT_MEMORY_LIST_PROVIDER.to_string(),
        DEN_PROMPT_MEMORY_PATCH => return DEN_PROMPT_MEMORY_PATCH_PROVIDER.to_string(),
        DEN_MEMORY_LIST_PROPOSALS => return DEN_MEMORY_LIST_PROPOSALS_PROVIDER.to_string(),
        DEN_MEMORY_READ_PROPOSAL => return DEN_MEMORY_READ_PROPOSAL_PROVIDER.to_string(),
        DEN_MEMORY_RESOLVE_PROPOSAL => return DEN_MEMORY_RESOLVE_PROPOSAL_PROVIDER.to_string(),
        DEN_MEMORY_APPLY_CORE_UPDATE => return DEN_MEMORY_APPLY_CORE_UPDATE_PROVIDER.to_string(),
        DEN_WORK_PLAN_LIST => return DEN_WORK_PLAN_LIST_PROVIDER.to_string(),
        DEN_WORK_PLAN_GET_STATUS => return DEN_WORK_PLAN_GET_STATUS_PROVIDER.to_string(),
        DEN_WORK_PLAN_UPDATE => return DEN_WORK_PLAN_UPDATE_PROVIDER.to_string(),
        DEN_WORK_PLAN_REQUEST_HANDOFF => return DEN_WORK_PLAN_REQUEST_HANDOFF_PROVIDER.to_string(),
        DEN_PLAN_MODE_ENTER => return DEN_PLAN_MODE_ENTER_PROVIDER.to_string(),
        DEN_PLAN_MODE_STATUS => return DEN_PLAN_MODE_STATUS_PROVIDER.to_string(),
        DEN_PLAN_MODE_RECORD_APPROVAL => return DEN_PLAN_MODE_RECORD_APPROVAL_PROVIDER.to_string(),
        DEN_PLAN_MODE_EXIT => return DEN_PLAN_MODE_EXIT_PROVIDER.to_string(),
        DEN_PLAN_MODE_CANCEL => return DEN_PLAN_MODE_CANCEL_PROVIDER.to_string(),
        _ => {}
    }
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        "den_tool".to_string()
    } else {
        safe
    }
}

pub fn builtin_den_tool_descriptors() -> Vec<DenToolDescriptor> {
    vec![
        descriptor(DEN_BEAR_GET_SELF, "About this bear", "Return Den's trusted profile for the current bear.", "bear", &["bear.read"], ALL_ROLES, empty_schema()),
        descriptor(DEN_USER_GET_CURRENT, "Current user", "Return Den's trusted profile for the current user in this interaction.", "session", &["user.current.read"], ALL_ROLES, empty_schema()),
        descriptor(DEN_BEAR_LIST_MEMBERS, "Bear members", "List users who have access to the current bear, with policy redaction.", "bear", &["bear.members.read"], ALL_ROLES, empty_schema()),
        descriptor(DEN_CAPABILITIES_LIST_SELF, "Available Den capabilities", "List Den-managed tools available to the current bear/session.", "session", &["capabilities.read"], ALL_ROLES, empty_schema()),
        descriptor(DEN_CHANNEL_GET_CONTEXT, "Channel context", "Return trusted Den/Codepool channel and session context for this interaction.", "session", &["channel.context.read"], ALL_ROLES, empty_schema()),
        descriptor(DEN_POLICY_GET_SELF, "Current policy", "Explain current user and bear policy for this interaction.", "session", &["policy.read"], ALL_ROLES, empty_schema()),
        descriptor(DEN_CONVERSATION_SET_TITLE, "Set conversation title", "Set the title of the current conversation. In some clients this may appear as the current chat or thread title. Does not change the conversation id, switch conversations, or write Bear memory.", "conversation", &["conversation.title.write"], CHAT_AND_PAIR_ROLES, set_conversation_title_schema()),
        descriptor(DEN_WEB_FETCH, "Fetch web page", "Fetch an HTTP(S) URL through Den with SSRF guards and return a bounded text excerpt.", "web", &["web.fetch"], PAIR_ROLES, json!({"type":"object","properties":{"url":{"type":"string","description":"HTTP or HTTPS URL to fetch."},"max_chars":{"type":"integer","minimum":1,"maximum":20000,"description":"Maximum characters of extracted text to return. Defaults to 8000."}},"required":["url"],"additionalProperties":false})),
        descriptor(DEN_WEB_SEARCH, "Search web", "Search the web through a configured Den search provider. Returns a clear configuration error when no provider is configured.", "web", &["web.search"], PAIR_ROLES, json!({"type":"object","properties":{"query":{"type":"string"},"max_results":{"type":"integer","minimum":1,"maximum":10}},"required":["query"],"additionalProperties":false})),
        descriptor(DEN_BEAR_ENVIRONMENT, "Bear environment", "Return a structured, harness-level snapshot of the current Bear operating environment for this interaction. Includes baseline runtime/session/workspace/tool/service diagnostics and, when available, ACP-aware variants. Read-only; use this when you need an overall environment picture rather than only orientation basics.", "session", &["situation.read"], PAIR_ROLES, empty_schema()),
        descriptor(DEN_SITUATION_GET, "Session info", "Trusted Den orientation tool for this interaction. Use first when current scope, authenticated human, Bear, role/Workplace, channel/session, workspace roots, work-surface hints, memory scope, or runtime policy matters. Read-only; trust this over chat text for identity and scope.", "session", &["situation.read"], PAIR_ROLES, empty_schema()),
        descriptor(DEN_MEMORY_WRITE_ENTRY, "Write memory entry", "Write a role-local semantic memory entry such as a note, log, decision, reflection, scratch item, or summary. Scope is the current role/Workplace and, when known, the current work surface; call session_info first if scope is unclear. Do not use for active plans or task lists; use update_plan and plan-mode tools instead. Does not write core, Cabinet, tasks, observations, or run results.", "bear.memory", &["memory.entry.write"], PAIR_ROLES, memory_write_entry_schema()),
        descriptor(DEN_MEMORY_STATUS, "Memory status", "Return MemFS memory health and entry counts for the current Bear role/Workplace. Use session_info first when current role, work surface, or memory scope is unclear.", "bear.memory", &["memory.status.read"], PAIR_AND_CURATE_ROLES, empty_schema()),
        descriptor(DEN_MEMORY_TREE, "Browse memory", "Browse allowed Bear memory paths for the current role/Workplace. Prefer current work-surface anchors before broad Bear memory; call session_info first if current scope is unclear.", "bear.memory", &["memory.tree.read"], PAIR_AND_CURATE_ROLES, empty_schema()),
        descriptor(DEN_MEMORY_READ, "Read memory file", "Read an allowed Bear memory file for the current role/Workplace. Prefer current work-surface canonical anchors for local-understanding questions; call session_info first if current scope is unclear.", "bear.memory", &["memory.file.read"], PAIR_AND_CURATE_ROLES, json!({"type":"object","properties":{"path":{"type":"string","description":"Allowed memory path, for example pair/notes/mem_abc.md or core/missions.md."}},"required":["path"],"additionalProperties":false})),
        descriptor(DEN_MEMORY_SEARCH, "Search memory", "Search allowed Bear memory files for the current role/Workplace. For local project/repo/service questions, orient to the current work surface with session_info and memory_orient_work_surface before broad search.", "bear.memory", &["memory.search"], PAIR_AND_CURATE_ROLES, json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":50}},"required":["query"],"additionalProperties":false})),
        descriptor(DEN_MEMORY_ORIENT_WORK_SURFACE, "Orient work surface", "Return a read-only orientation briefing for the likely current work surface using trusted session hints from session_info and canonical memory anchor paths when available. Use before broad memory search for local project/repo/service questions.", "bear.memory", &["memory.tree.read", "memory.file.read"], PAIR_AND_CURATE_ROLES, empty_schema()),
        descriptor(DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD, "Create work-surface scaffold", "Create a minimal work-surface scaffold in Bear memory and register it in the work-surface index. Mutates memory; call session_info and memory_orient_work_surface first unless the user explicitly names the work surface.", "bear.memory", &["memory.write", "memory.core.write"], PAIR_ROLES, json!({"type":"object","properties":{"work_surface_slug":{"type":"string","minLength":1,"maxLength":80},"work_surface_name":{"type":"string","minLength":1,"maxLength":200},"overview":{"type":"string","minLength":1,"maxLength":20000},"glossary":{"type":"string","maxLength":20000},"current_understanding":{"type":"string","maxLength":20000}},"required":["work_surface_slug", "work_surface_name", "overview"],"additionalProperties":false})),
        descriptor(DEN_MEMORY_REQUEST_REVIEW, "Request memory review", "Request Reflection/curate review of role-local memory without writing shared memory directly. Use for role/Workplace-local material that may deserve broader Bear-global review; call session_info first if scope/provenance is unclear.", "bear.memory", &["memory.review.request"], PAIR_ROLES, memory_request_review_schema()),
        descriptor(DEN_PROMPT_MEMORY_UPSERT, "Upsert prompt memory block", "Create or replace a Den-owned prompt memory block for the current bear role. Use this for editable runtime prompt memory, not semantic memory notes.", "bear.memory", &["memory.entry.write"], PAIR_ROLES, prompt_memory_upsert_schema()),
        descriptor(DEN_PROMPT_MEMORY_LIST, "List prompt memory blocks", "List Den-owned prompt memory blocks for the current bear role.", "bear.memory", &["memory.status.read"], PAIR_ROLES, json!({"type":"object","properties":{"include_archived":{"type":"boolean"},"scope":{"type":"string","enum":["bear_wide","role_local","work_surface","session"]},"block_type":{"type":"string","enum":["role_guidance","work_surface_context","session_focus","user_instruction"]},"work_surface":{"type":"string","maxLength":500},"session_id":{"type":"string","maxLength":200}},"additionalProperties":false})),
        descriptor(DEN_PROMPT_MEMORY_PATCH, "Patch prompt memory block", "Update lifecycle/content fields for an existing Den-owned prompt memory block.", "bear.memory", &["memory.entry.write"], PAIR_ROLES, prompt_memory_patch_schema()),
        descriptor(DEN_MEMORY_LIST_PROPOSALS, "List memory proposals", "List memory review proposals for this Bear.", "bear.memory", &["memory.proposal.read"], CURATE_ROLES, json!({"type":"object","properties":{"status":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false})),
        descriptor(DEN_MEMORY_READ_PROPOSAL, "Read memory proposal", "Read one memory review proposal with source pointers and status.", "bear.memory", &["memory.proposal.read"], CURATE_ROLES, json!({"type":"object","properties":{"proposal_id":{"type":"string","format":"uuid"}},"required":["proposal_id"],"additionalProperties":false})),
        descriptor(DEN_MEMORY_RESOLVE_PROPOSAL, "Resolve memory proposal", "Resolve a memory review proposal without applying shared-memory writes.", "bear.memory", &["memory.proposal.resolve"], CURATE_ROLES, json!({"type":"object","properties":{"proposal_id":{"type":"string","format":"uuid"},"status":{"enum":["rejected","retained_local","deferred","superseded","needs_human_review"]},"review_notes":{"type":"string"},"decision_summary":{"type":"string"}},"required":["proposal_id","status"],"additionalProperties":false})),
        descriptor(DEN_MEMORY_APPLY_CORE_UPDATE, "Apply core memory update", "Apply a reviewed update to allowed core memory paths with provenance.", "bear.memory", &["memory.core.write"], CURATE_ROLES, json!({"type":"object","properties":{"proposal_id":{"type":"string","format":"uuid"},"target_path":{"type":"string"},"mode":{"enum":["append_section","create_file","replace_text"]},"title":{"type":"string"},"body":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"},"review_notes":{"type":"string"}},"required":["proposal_id","target_path","mode"],"additionalProperties":false})),
        descriptor(DEN_SKILL_PROPOSE, "Propose skill", "Capture a durable skill proposal for curate review without installing it directly.", "bear.skills", &["skill.proposal.write"], ALL_ROLES, json!({"type":"object","properties":{"skill_name":{"type":"string"},"skill_version":{"type":"string"},"rationale":{"type":"string"},"proposed_content":{"type":"string"},"desired_roles":{"type":"array","items":{"enum":ALL_ROLES}},"provenance":{"type":"object"}},"required":["skill_name","rationale","proposed_content"],"additionalProperties":false})),
        descriptor(DEN_SKILL_APPROVE_PROPOSAL, "Approve skill proposal", "Approve a pending skill proposal, update the manifest, and queue reconciliation for affected roles.", "bear.skills", &["skill.proposal.approve"], CURATE_ROLES, json!({"type":"object","properties":{"proposal_id":{"type":"string","format":"uuid"},"skill_name":{"type":"string"},"skill_version":{"type":"string"},"applies_to_roles":{"type":"array","items":{"enum":ALL_ROLES},"minItems":1},"review_notes":{"type":"string"}},"required":["proposal_id","applies_to_roles"],"additionalProperties":false})),
        descriptor(DEN_SKILL_REJECT_PROPOSAL, "Reject skill proposal", "Reject a pending skill proposal with reviewer metadata and a rejection reason.", "bear.skills", &["skill.proposal.reject"], CURATE_ROLES, json!({"type":"object","properties":{"proposal_id":{"type":"string","format":"uuid"},"rejection_reason":{"type":"string"},"review_notes":{"type":"string"}},"required":["proposal_id","rejection_reason"],"additionalProperties":false})),
        descriptor(DEN_WORK_PLAN_LIST, "List plans", "List visible Bear-level planning state, including live activity plans, submitted workplan gates, and saved workplan artifacts where available. Call session_info first if current thread/session/work-surface scope is unclear.", "bear.activity", &["work_plan.read"], WORK_PLAN_READ_ROLES, json!({"type":"object","properties":{"status":{"type":"array","items":{"enum":["active","blocked","completed","cancelled","archived"]}},"owner_role":{"enum":ALL_ROLES},"include_archived":{"type":"boolean"},"include_completed":{"type":"boolean"},"include_plan_mode":{"type":"boolean"},"include_artifacts":{"type":"boolean"}},"additionalProperties":false})),
        descriptor(DEN_WORK_PLAN_GET_STATUS, "Get work plan status", "Return current status for one visible Den activity plan or this session's active plan. Use to orient before continuing, updating, or handing off plan work; call session_info first if session scope is unclear.", "bear.activity", &["work_plan.read"], WORK_PLAN_READ_ROLES, json!({"type":"object","properties":{"plan_id":{"type":"string","format":"uuid"},"source_acp_session_id":{"type":"string"},"source_conversation_id":{"type":"string"}},"additionalProperties":false})),
        descriptor(DEN_WORK_PLAN_UPDATE, "Update visible plan", "Create or update the current role's live visible activity plan. Use this when the user asks to create, show, update, or execute a plan/task list. This is active work state, not semantic memory; call session_info first if current session/work-surface scope is unclear.", "bear.activity", &["work_plan.write"], WORK_PLAN_UPDATE_ROLES, json!({"type":"object","properties":{"plan_id":{"type":"string","format":"uuid"},"expected_version":{"type":"integer","minimum":1},"title":{"type":"string"},"summary":{"type":"string"},"visibility":{"enum":["private_to_role","same_user","bear_visible","handoff_requested"]},"status":{"enum":["active","blocked","completed","cancelled","archived"]},"items":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"title":{"type":"string"},"summary":{"type":"string"},"status":{"enum":["pending","in_progress","blocked","completed","cancelled"]},"blocked_reason":{"type":"string"},"source_refs":{"type":"array","items":{"type":"string"}}},"required":["id","title","status"],"additionalProperties":false}},"workspace_context":{"type":"object"}},"required":["title","visibility","status","items"],"additionalProperties":false})),
        descriptor(DEN_WORK_PLAN_REQUEST_HANDOFF, "Request task handoff", "Request conversion of selected live activity plan items into a schema-validated task intent for curate review.", "bear.activity", &["work_plan.handoff.request"], CHAT_AND_PAIR_ROLES, json!({"type":"object","properties":{"plan_id":{"type":"string","format":"uuid"},"item_ids":{"type":"array","items":{"type":"string"}},"title":{"type":"string"},"summary":{"type":"string"},"requested_outcome":{"type":"string"},"constraints":{"type":"array","items":{"type":"string"}},"allowed_tools_hint":{"type":"array","items":{"type":"string"}}},"required":["plan_id","item_ids","title","summary","requested_outcome"],"additionalProperties":false})),
        descriptor(DEN_PLAN_MODE_ENTER, "Enter planning mode", "Enter ACP pair workplan mode and reflect that mode in the ACP session UI. Use this when the user asks to enter planning mode.", "bear.workplan", &["plan_mode.enter"], PAIR_ROLES, json!({"type":"object","properties":{"reason":{"type":"string"},"previous_permission_mode":{"type":"string"}},"additionalProperties":false})),
        descriptor(DEN_PLAN_MODE_STATUS, "Get plan mode status", "Return the current ACP pair workplan gate for this session, if any.", "bear.workplan", &["plan_mode.read"], PAIR_ROLES, empty_schema()),
        descriptor(DEN_PLAN_MODE_RECORD_APPROVAL, "Record plan approval", "Record explicit approval from the authenticated human for the currently submitted implementation workplan. Use only when the user clearly approves the current plan in this conversation, for example 'go ahead', 'approved', or 'proceed'.", "bear.workplan", &["plan_mode.approve"], PAIR_ROLES, json!({"type":"object","properties":{"plan_mode_id":{"type":"string","format":"uuid"},"approval_text":{"type":"string","description":"The user's approval text that prompted this tool call."}},"required":["approval_text"],"additionalProperties":false})),
        descriptor(DEN_PLAN_MODE_EXIT, "Submit implementation plan", "Submit a markdown implementation workplan artifact for user approval. This is for durable implementation workplans, not for the live visible task list; use update_plan for visible activity planning.", "bear.workplan", &["plan_mode.exit"], PAIR_ROLES, json!({"type":"object","properties":{"plan_mode_id":{"type":"string","format":"uuid"},"title":{"type":"string"},"body":{"type":"string"}},"required":["title","body"],"additionalProperties":false})),
        descriptor(DEN_PLAN_MODE_CANCEL, "Cancel plan mode", "Cancel the current ACP pair workplan gate without approving implementation.", "bear.workplan", &["plan_mode.cancel"], PAIR_ROLES, json!({"type":"object","properties":{"plan_mode_id":{"type":"string","format":"uuid"}},"additionalProperties":false})),
        descriptor(DEN_TASK_WRITE_INTENT, "Write task intent", "Write a schema-validated task intent from chat or pair for later curate review.", "bear.tasks", &["task.intent.write"], CHAT_AND_PAIR_ROLES, json!({"type":"object","properties":{"title":{"type":"string"},"summary":{"type":"string"},"requested_outcome":{"type":"string"},"constraints":{"type":"array","items":{"type":"string"}},"allowed_tools_hint":{"type":"array","items":{"type":"string"}},"source_reference":{"type":"object"}},"required":["title","summary","requested_outcome"],"additionalProperties":false})),
        descriptor(DEN_TASK_APPROVE_INTENT, "Approve task intent", "Approve a chat/pair task intent, write the canonical core task, and update source intent audit metadata.", "bear.tasks", &["task.intent.approve"], CURATE_ROLES, json!({"type":"object","properties":{"source_intent_path":{"type":"string"},"task_id":{"type":"string"},"title":{"type":"string"},"approved_scope":{"type":"object"},"allowed_tools":{"type":"array","items":{"type":"string"}},"expires_at":{"type":"string"},"review_notes":{"type":"string"}},"required":["source_intent_path","task_id","title","approved_scope","allowed_tools"],"additionalProperties":false})),
        descriptor(DEN_TASK_REJECT_INTENT, "Reject task intent", "Reject a chat/pair task intent and update source intent audit metadata with the rejection reason.", "bear.tasks", &["task.intent.reject"], CURATE_ROLES, json!({"type":"object","properties":{"source_intent_path":{"type":"string"},"rejection_reason":{"type":"string"},"review_notes":{"type":"string"}},"required":["source_intent_path","rejection_reason"],"additionalProperties":false})),
        descriptor(DEN_CORE_WRITE_RESULT_SUMMARY, "Write core result summary", "Write a curate-reviewed summary of work results into shared core memory through Den-controlled validation.", "bear.core", &["core.result_summary.write"], CURATE_ROLES, json!({"type":"object","properties":{"task_id":{"type":"string"},"run_id":{"type":"string"},"summary":{"type":"string"},"durable_learnings":{"type":"array","items":{"type":"string"}},"source_result_path":{"type":"string"}},"required":["task_id","summary"],"additionalProperties":false})),
        descriptor(DEN_OBSERVATION_WRITE, "Write observation", "Write a schema-validated inbound observation from a Den-delivered watch event.", "bear.observations", &["observation.write"], WATCH_ROLES, json!({"type":"object","properties":{"observation_id":{"type":"string"},"summary":{"type":"string"},"salience":{"type":"string"},"payload_ref":{"type":"string"},"source":{"type":"object"}},"required":["summary"],"additionalProperties":false})),
        descriptor(DEN_RUN_WRITE_RESULT, "Write run result", "Write a schema-validated work run result under the active Den-issued run context.", "bear.runs", &["run.result.write"], WORK_ROLES, json!({"type":"object","properties":{"task_id":{"type":"string"},"run_id":{"type":"string"},"status":{"enum":["succeeded","failed","partial"]},"summary":{"type":"string"},"result":{"type":"object"},"follow_up":{"type":"array","items":{"type":"string"}}},"required":["task_id","run_id","status","summary"],"additionalProperties":false})),
    ]
}

pub fn builtin_den_tool_descriptors_for_role(role: BearAgentRole) -> Vec<DenToolDescriptor> {
    builtin_den_tool_descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.allows_role(role))
        .collect()
}

pub fn builtin_den_tool_descriptor_for_provider_name(provider_name: &str) -> Option<DenToolDescriptor> {
    builtin_den_tool_descriptors().into_iter().find(|descriptor| {
        descriptor.provider_name == provider_name
            || descriptor.provider_aliases.contains(&provider_name)
            || descriptor.name == provider_name
    })
}

pub fn is_builtin_den_tool(tool_name: &str) -> bool {
    builtin_den_tool_descriptor_for_provider_name(tool_name).is_some()
}

pub fn provider_aliases_for_tool(name: &str) -> &'static [&'static str] {
    match name {
        DEN_WEB_FETCH => &[DEN_WEB_FETCH_LEGACY_PROVIDER],
        DEN_SITUATION_GET => &[DEN_SITUATION_GET_LEGACY_PROVIDER],
        DEN_MEMORY_TREE => &[DEN_MEMORY_TREE_LEGACY_PROVIDER],
        _ => &[],
    }
}

fn den_tool_description(name: &'static str, description: &'static str) -> &'static str {
    let guidance = match name {
        DEN_SITUATION_GET => None,
        DEN_CONVERSATION_SET_TITLE => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::Conversation,
            side_effect: ToolSideEffectKind::ConversationMetadata,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_WEB_FETCH | DEN_WEB_SEARCH => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::ExternalWeb,
            side_effect: ToolSideEffectKind::ExternalNetwork,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_BEAR_ENVIRONMENT => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::Conversation,
            side_effect: ToolSideEffectKind::ReadOnly,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_MEMORY_WRITE_ENTRY => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::BearRoleMemory,
            side_effect: ToolSideEffectKind::WritesMemory,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_MEMORY_STATUS
        | DEN_MEMORY_TREE
        | DEN_MEMORY_READ
        | DEN_MEMORY_SEARCH
        | DEN_MEMORY_ORIENT_WORK_SURFACE => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::BearRoleMemory,
            side_effect: ToolSideEffectKind::ReadOnly,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::BearRoleMemory,
            side_effect: ToolSideEffectKind::WritesMemory,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_MEMORY_REQUEST_REVIEW => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::BearRoleMemory,
            side_effect: ToolSideEffectKind::WritesMemory,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_WORK_PLAN_LIST | DEN_WORK_PLAN_GET_STATUS => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::CurrentSession,
            side_effect: ToolSideEffectKind::ReadOnly,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_WORK_PLAN_UPDATE | DEN_WORK_PLAN_REQUEST_HANDOFF => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::CurrentSession,
            side_effect: ToolSideEffectKind::ActiveWorkState,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_PLAN_MODE_ENTER
        | DEN_PLAN_MODE_STATUS
        | DEN_PLAN_MODE_RECORD_APPROVAL
        | DEN_PLAN_MODE_EXIT
        | DEN_PLAN_MODE_CANCEL => Some(ToolDescriptorGuidance {
            scope: ToolScopeKind::CurrentSession,
            side_effect: ToolSideEffectKind::ActiveWorkState,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        }),
        DEN_SKILL_PROPOSE | DEN_SKILL_APPROVE_PROPOSAL | DEN_SKILL_REJECT_PROPOSAL => {
            Some(ToolDescriptorGuidance {
                scope: ToolScopeKind::CurrentSession,
                side_effect: ToolSideEffectKind::SkillGovernance,
                orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
            })
        }
        _ => None,
    };
    let Some(guidance) = guidance else {
        return description;
    };
    Box::leak(
        format!(
            "{} {}",
            description,
            render_tool_descriptor_guidance(guidance)
        )
        .into_boxed_str(),
    )
}

fn descriptor(
    name: &'static str,
    label: &'static str,
    description: &'static str,
    scope: &'static str,
    permissions: &'static [&'static str],
    allowed_roles: &'static [&'static str],
    input_schema: Value,
) -> DenToolDescriptor {
    DenToolDescriptor {
        name,
        provider_name: provider_safe_tool_name(name),
        provider_aliases: provider_aliases_for_tool(name),
        label,
        description: den_tool_description(name, description),
        kind: "server_tool",
        provider: "den",
        execution_target: "den",
        scope,
        domain: tool_domain(name),
        content_class: tool_content_class(name),
        availability: "available",
        permissions,
        allowed_roles,
        approval_policy: "never",
        display: den_tool_display(name, label).to_json(),
        input_schema,
    }
}

pub fn den_tool_display(name: &'static str, label: &'static str) -> AcpToolDisplayDescriptor {
    match name {
        DEN_CONVERSATION_SET_TITLE => AcpToolDisplayDescriptor {
            label,
            category: "conversation",
            progress_verb: "Setting conversation title",
            complete_verb: "Set conversation title",
            target_arg_keys: &["title"],
            sensitive_arg_keys: &[],
            approval_summary: "Update the visible conversation title.",
        },
        DEN_WEB_FETCH => AcpToolDisplayDescriptor {
            label,
            category: "web",
            progress_verb: "Fetching",
            complete_verb: "Fetched",
            target_arg_keys: &["url"],
            sensitive_arg_keys: &[],
            approval_summary: "Fetch this URL with Den web safeguards.",
        },
        DEN_WEB_SEARCH => AcpToolDisplayDescriptor {
            label,
            category: "web",
            progress_verb: "Searching web for",
            complete_verb: "Searched web for",
            target_arg_keys: &["query"],
            sensitive_arg_keys: &[],
            approval_summary: "Search the web through the configured Den provider.",
        },
        DEN_BEAR_ENVIRONMENT => AcpToolDisplayDescriptor {
            label,
            category: "orientation",
            progress_verb: "Inspecting bear environment",
            complete_verb: "Inspected bear environment",
            target_arg_keys: &[],
            sensitive_arg_keys: &[],
            approval_summary: "Read a structured snapshot of the current Bear runtime environment.",
        },
        DEN_SITUATION_GET => AcpToolDisplayDescriptor {
            label,
            category: "orientation",
            progress_verb: "Checking session info",
            complete_verb: "Checked session info",
            target_arg_keys: &[],
            sensitive_arg_keys: &[],
            approval_summary:
                "Read trusted session, Bear, human, policy, and workspace orientation.",
        },
        DEN_MEMORY_WRITE_ENTRY => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Writing memory entry",
            complete_verb: "Wrote memory entry",
            target_arg_keys: &["title", "path"],
            sensitive_arg_keys: &["body", "content"],
            approval_summary: "Write a role-local memory entry with provenance.",
        },
        DEN_MEMORY_STATUS => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Checking memory status",
            complete_verb: "Checked memory status",
            target_arg_keys: &[],
            sensitive_arg_keys: &[],
            approval_summary: "Read memory health and counts.",
        },
        DEN_MEMORY_TREE => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Browsing memory",
            complete_verb: "Browsed memory",
            target_arg_keys: &[],
            sensitive_arg_keys: &[],
            approval_summary: "Browse allowed memory paths.",
        },
        DEN_MEMORY_READ => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Reading memory",
            complete_verb: "Read memory",
            target_arg_keys: &["path"],
            sensitive_arg_keys: &[],
            approval_summary: "Read this allowed memory file.",
        },
        DEN_MEMORY_SEARCH => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Searching memory for",
            complete_verb: "Searched memory for",
            target_arg_keys: &["query"],
            sensitive_arg_keys: &[],
            approval_summary: "Search allowed Bear memory.",
        },
        DEN_MEMORY_ORIENT_WORK_SURFACE => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Orienting work surface",
            complete_verb: "Oriented work surface",
            target_arg_keys: &[],
            sensitive_arg_keys: &[],
            approval_summary: "Read work-surface memory anchors and orientation.",
        },
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Creating work-surface scaffold",
            complete_verb: "Created work-surface scaffold",
            target_arg_keys: &["work_surface_slug", "work_surface_name"],
            sensitive_arg_keys: &["overview", "glossary", "current_understanding"],
            approval_summary: "Create canonical memory scaffold for this work surface.",
        },
        DEN_MEMORY_REQUEST_REVIEW => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Requesting memory review",
            complete_verb: "Requested memory review",
            target_arg_keys: &["title"],
            sensitive_arg_keys: &["summary", "rationale", "proposed_content", "proposed_patch"],
            approval_summary: "Ask curate to review role-local memory.",
        },
        DEN_MEMORY_LIST_PROPOSALS => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Listing memory proposals",
            complete_verb: "Listed memory proposals",
            target_arg_keys: &["status"],
            sensitive_arg_keys: &[],
            approval_summary: "List memory review proposals.",
        },
        DEN_MEMORY_READ_PROPOSAL => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Reading memory proposal",
            complete_verb: "Read memory proposal",
            target_arg_keys: &["proposal_id"],
            sensitive_arg_keys: &[],
            approval_summary: "Read this memory review proposal.",
        },
        DEN_MEMORY_RESOLVE_PROPOSAL => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Resolving memory proposal",
            complete_verb: "Resolved memory proposal",
            target_arg_keys: &["proposal_id", "status"],
            sensitive_arg_keys: &["review_notes", "decision_summary"],
            approval_summary: "Record a curate decision for this memory proposal.",
        },
        DEN_MEMORY_APPLY_CORE_UPDATE => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Applying core memory update",
            complete_verb: "Applied core memory update",
            target_arg_keys: &["target_path", "mode"],
            sensitive_arg_keys: &["body", "old_text", "new_text", "review_notes"],
            approval_summary: "Apply a reviewed update to core memory.",
        },
        DEN_SKILL_PROPOSE => AcpToolDisplayDescriptor {
            label,
            category: "skills",
            progress_verb: "Proposing skill",
            complete_verb: "Proposed skill",
            target_arg_keys: &["skill_name", "skill_version"],
            sensitive_arg_keys: &["proposed_content"],
            approval_summary: "Create a skill proposal for curate review.",
        },
        DEN_SKILL_APPROVE_PROPOSAL => AcpToolDisplayDescriptor {
            label,
            category: "skills",
            progress_verb: "Approving skill proposal",
            complete_verb: "Approved skill proposal",
            target_arg_keys: &["proposal_id", "skill_name"],
            sensitive_arg_keys: &["review_notes"],
            approval_summary: "Approve this skill proposal.",
        },
        DEN_SKILL_REJECT_PROPOSAL => AcpToolDisplayDescriptor {
            label,
            category: "skills",
            progress_verb: "Rejecting skill proposal",
            complete_verb: "Rejected skill proposal",
            target_arg_keys: &["proposal_id"],
            sensitive_arg_keys: &["rejection_reason", "review_notes"],
            approval_summary: "Reject this skill proposal.",
        },
        DEN_WORK_PLAN_LIST => AcpToolDisplayDescriptor {
            label,
            category: "plan",
            progress_verb: "Listing plans",
            complete_verb: "Listed plans",
            target_arg_keys: &["owner_role"],
            sensitive_arg_keys: &[],
            approval_summary: "Read visible planning state.",
        },
        DEN_WORK_PLAN_GET_STATUS => AcpToolDisplayDescriptor {
            label,
            category: "plan",
            progress_verb: "Checking plan status",
            complete_verb: "Checked plan status",
            target_arg_keys: &["plan_id"],
            sensitive_arg_keys: &[],
            approval_summary: "Read visible plan status.",
        },
        DEN_WORK_PLAN_UPDATE => AcpToolDisplayDescriptor {
            label,
            category: "plan",
            progress_verb: "Updating visible plan",
            complete_verb: "Updated visible plan",
            target_arg_keys: &["title", "plan_id"],
            sensitive_arg_keys: &["summary", "items", "workspace_context"],
            approval_summary: "Update active visible work state.",
        },
        DEN_WORK_PLAN_REQUEST_HANDOFF => AcpToolDisplayDescriptor {
            label,
            category: "plan",
            progress_verb: "Requesting work handoff",
            complete_verb: "Requested work handoff",
            target_arg_keys: &["title", "plan_id"],
            sensitive_arg_keys: &["summary", "requested_outcome", "constraints"],
            approval_summary: "Request conversion of plan items into task intent.",
        },
        DEN_PLAN_MODE_ENTER => AcpToolDisplayDescriptor {
            label,
            category: "plan",
            progress_verb: "Entering planning mode",
            complete_verb: "Entered planning mode",
            target_arg_keys: &[],
            sensitive_arg_keys: &["reason"],
            approval_summary: "Enter ACP planning mode.",
        },
        DEN_PLAN_MODE_STATUS => AcpToolDisplayDescriptor {
            label,
            category: "plan",
            progress_verb: "Checking planning mode",
            complete_verb: "Checked planning mode",
            target_arg_keys: &[],
            sensitive_arg_keys: &[],
            approval_summary: "Read current planning gate state.",
        },
        DEN_PLAN_MODE_RECORD_APPROVAL => AcpToolDisplayDescriptor {
            label,
            category: "plan",
            progress_verb: "Recording plan approval",
            complete_verb: "Recorded plan approval",
            target_arg_keys: &["plan_mode_id"],
            sensitive_arg_keys: &["approval_text"],
            approval_summary: "Record explicit human approval for the submitted plan.",
        },
        DEN_PLAN_MODE_EXIT => AcpToolDisplayDescriptor {
            label,
            category: "plan",
            progress_verb: "Submitting implementation plan",
            complete_verb: "Submitted implementation plan",
            target_arg_keys: &["title"],
            sensitive_arg_keys: &["body"],
            approval_summary: "Submit an implementation workplan for approval.",
        },
        DEN_PLAN_MODE_CANCEL => AcpToolDisplayDescriptor {
            label,
            category: "plan",
            progress_verb: "Cancelling planning mode",
            complete_verb: "Cancelled planning mode",
            target_arg_keys: &["plan_mode_id"],
            sensitive_arg_keys: &[],
            approval_summary: "Cancel the current planning gate.",
        },
        DEN_TASK_WRITE_INTENT => AcpToolDisplayDescriptor {
            label,
            category: "tasks",
            progress_verb: "Writing task intent",
            complete_verb: "Wrote task intent",
            target_arg_keys: &["title"],
            sensitive_arg_keys: &["summary", "requested_outcome", "constraints"],
            approval_summary: "Write a task intent for curate review.",
        },
        DEN_TASK_APPROVE_INTENT => AcpToolDisplayDescriptor {
            label,
            category: "tasks",
            progress_verb: "Approving task intent",
            complete_verb: "Approved task intent",
            target_arg_keys: &["task_id", "title"],
            sensitive_arg_keys: &["approved_scope", "review_notes"],
            approval_summary: "Approve this task intent.",
        },
        DEN_TASK_REJECT_INTENT => AcpToolDisplayDescriptor {
            label,
            category: "tasks",
            progress_verb: "Rejecting task intent",
            complete_verb: "Rejected task intent",
            target_arg_keys: &["source_intent_path"],
            sensitive_arg_keys: &["rejection_reason", "review_notes"],
            approval_summary: "Reject this task intent.",
        },
        DEN_CORE_WRITE_RESULT_SUMMARY => AcpToolDisplayDescriptor {
            label,
            category: "memory",
            progress_verb: "Writing core result summary",
            complete_verb: "Wrote core result summary",
            target_arg_keys: &["task_id", "run_id"],
            sensitive_arg_keys: &["summary", "durable_learnings"],
            approval_summary: "Write a reviewed result summary to core memory.",
        },
        DEN_OBSERVATION_WRITE => AcpToolDisplayDescriptor {
            label,
            category: "observations",
            progress_verb: "Writing observation",
            complete_verb: "Wrote observation",
            target_arg_keys: &["observation_id"],
            sensitive_arg_keys: &["summary", "payload_ref", "source"],
            approval_summary: "Write a watch observation.",
        },
        DEN_RUN_WRITE_RESULT => AcpToolDisplayDescriptor {
            label,
            category: "runs",
            progress_verb: "Writing run result",
            complete_verb: "Wrote run result",
            target_arg_keys: &["task_id", "run_id", "status"],
            sensitive_arg_keys: &["summary", "result", "follow_up"],
            approval_summary: "Write a work run result.",
        },
        _ => AcpToolDisplayDescriptor {
            label,
            category: "den",
            progress_verb: "Using",
            complete_verb: "Used",
            target_arg_keys: &[],
            sensitive_arg_keys: &[],
            approval_summary: "Use this Den tool.",
        },
    }
}

fn tool_domain(name: &str) -> &'static str {
    match name {
        DEN_PLAN_MODE_ENTER
        | DEN_PLAN_MODE_STATUS
        | DEN_PLAN_MODE_RECORD_APPROVAL
        | DEN_PLAN_MODE_EXIT
        | DEN_PLAN_MODE_CANCEL => "workplan",
        DEN_WORK_PLAN_LIST
        | DEN_WORK_PLAN_GET_STATUS
        | DEN_WORK_PLAN_UPDATE
        | DEN_WORK_PLAN_REQUEST_HANDOFF => "activity",
        DEN_MEMORY_WRITE_ENTRY
        | DEN_MEMORY_STATUS
        | DEN_MEMORY_TREE
        | DEN_MEMORY_READ
        | DEN_MEMORY_SEARCH
        | DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD
        | DEN_MEMORY_REQUEST_REVIEW
        | DEN_MEMORY_LIST_PROPOSALS
        | DEN_MEMORY_READ_PROPOSAL
        | DEN_MEMORY_RESOLVE_PROPOSAL
        | DEN_MEMORY_APPLY_CORE_UPDATE => "memory",
        DEN_CONVERSATION_SET_TITLE
        | DEN_WEB_FETCH
        | DEN_WEB_SEARCH
        | DEN_BEAR_ENVIRONMENT
        | DEN_SITUATION_GET => "execution",
        _ => "execution",
    }
}

fn tool_content_class(name: &str) -> Option<&'static str> {
    match name {
        DEN_MEMORY_WRITE_ENTRY => Some("semantic_memory"),
        DEN_MEMORY_CREATE_WORK_SURFACE_SCAFFOLD => Some("semantic_memory"),
        DEN_MEMORY_REQUEST_REVIEW => Some("semantic_memory"),
        DEN_BEAR_ENVIRONMENT => Some("activity_status"),
        DEN_PLAN_MODE_EXIT => Some("workplan_artifact"),
        DEN_WORK_PLAN_UPDATE => Some("activity_status"),
        DEN_WORK_PLAN_REQUEST_HANDOFF => Some("task_intent"),
        DEN_MEMORY_APPLY_CORE_UPDATE => Some("core_update"),
        DEN_OBSERVATION_WRITE => Some("observation"),
        DEN_RUN_WRITE_RESULT => Some("run_result"),
        _ => None,
    }
}

fn memory_request_review_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "source_paths": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 20 },
            "title": { "type": "string", "minLength": 1, "maxLength": 200 },
            "summary": { "type": "string", "minLength": 1, "maxLength": 4000 },
            "rationale": { "type": "string", "maxLength": 4000 },
            "suggested_action": { "type": "string", "enum": ["unspecified", "summarize_into_core", "promote_to_core", "cabinet_update", "skill_review", "retain_role_local", "delete_after_review", "human_review", "archive_index", "task_context"] },
            "target_ref": { "type": "string", "maxLength": 500 },
            "refs": { "type": "object" },
            "sensitivity": { "type": "string", "enum": ["normal", "person", "secret_risk", "external_untrusted", "unknown"] },
            "requires_human": { "type": "boolean" },
            "proposed_content": { "type": "string", "maxLength": 20000 },
            "proposed_patch": { "type": "string", "maxLength": 20000 }
        },
        "required": ["source_paths", "title", "summary"],
        "additionalProperties": false
    })
}

fn empty_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

fn set_conversation_title_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "minLength": 1,
                "maxLength": 120,
                "description": "New title for the current conversation."
            }
        },
        "required": ["title"],
        "additionalProperties": false
    })
}

fn prompt_memory_upsert_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "block_id": { "type": "string", "minLength": 1, "maxLength": 200 },
            "scope": { "type": "string", "enum": ["bear_wide", "role_local", "work_surface", "session"] },
            "block_type": { "type": "string", "enum": ["role_guidance", "work_surface_context", "session_focus", "user_instruction"] },
            "state": { "type": "string", "enum": ["draft", "active", "superseded", "archived"] },
            "work_surface": { "type": "string", "maxLength": 500 },
            "session_id": { "type": "string", "maxLength": 200 },
            "title": { "type": "string", "minLength": 1, "maxLength": 200 },
            "body": { "type": "string", "minLength": 1, "maxLength": 50000 },
            "priority": { "type": "integer", "minimum": -1000, "maximum": 1000 },
            "supersedes_block_id": { "type": "string", "maxLength": 200 },
            "metadata": { "type": "object" }
        },
        "required": ["block_id", "scope", "block_type", "title", "body"],
        "additionalProperties": false
    })
}

fn prompt_memory_patch_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "block_id": { "type": "string", "minLength": 1, "maxLength": 200 },
            "state": { "type": "string", "enum": ["draft", "active", "superseded", "archived"] },
            "title": { "type": "string", "minLength": 1, "maxLength": 200 },
            "body": { "type": "string", "minLength": 1, "maxLength": 50000 },
            "priority": { "type": "integer", "minimum": -1000, "maximum": 1000 },
            "supersedes_block_id": { "type": "string", "maxLength": 200 },
            "metadata": { "type": "object" }
        },
        "required": ["block_id", "title", "body"],
        "additionalProperties": false
    })
}

fn memory_write_entry_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["note", "log", "decision", "reflection", "scratch", "summary", "plan"]
            },
            "title": { "type": "string", "minLength": 1, "maxLength": 200 },
            "body": { "type": "string", "minLength": 1, "maxLength": 50000 },
            "tags": {
                "type": "array",
                "items": { "type": "string", "minLength": 1, "maxLength": 80 },
                "maxItems": 20
            },
            "refs": {
                "type": "object",
                "properties": {
                    "people": { "type": "array", "items": { "type": "string" }, "maxItems": 20 },
                    "missions": { "type": "array", "items": { "type": "string" }, "maxItems": 20 },
                    "knowledge": { "type": "array", "items": { "type": "string" }, "maxItems": 20 },
                    "cabinet": { "type": "array", "items": { "type": "string" }, "maxItems": 20 },
                    "artifacts": { "type": "array", "items": { "type": "string" }, "maxItems": 20 },
                    "tasks": { "type": "array", "items": { "type": "string" }, "maxItems": 20 }
                },
                "additionalProperties": false
            },
            "lifecycle": {
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["role-local", "core-candidate", "cabinet-candidate"] },
                    "retention": { "type": "string", "enum": ["session", "short", "durable", "archive"] },
                    "promotion": { "type": "string", "enum": ["none", "maybe", "proposed"] },
                    "status": { "type": "string", "enum": ["active", "superseded", "stale", "archived"] }
                },
                "additionalProperties": false
            },
            "source": { "type": "object" },
            "content_class": {
                "type": "string",
                "enum": ["semantic_memory", "workplan_artifact", "activity_status", "task_intent", "run_result", "observation", "core_update", "cabinet_write"]
            },
            "domain": {
                "type": "string",
                "enum": ["workplan", "activity", "memory", "execution"]
            },
            "semantic_confirmation_token": { "type": "string", "minLength": 1, "maxLength": 2000 }
        },
        "required": ["kind", "title", "body"],
        "additionalProperties": false
    })
}

impl DenToolDescriptor {
    pub fn allows_role(&self, role: BearAgentRole) -> bool {
        self.allowed_roles.iter().any(|allowed| *allowed == role.as_str())
    }
}

impl<'de> Deserialize<'de> for DenToolDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawDenToolDescriptor {
            name: String,
            provider_name: String,
            provider_aliases: Vec<String>,
            label: String,
            description: String,
            kind: String,
            provider: String,
            execution_target: String,
            scope: String,
            domain: String,
            content_class: Option<String>,
            availability: String,
            permissions: Vec<String>,
            allowed_roles: Vec<String>,
            approval_policy: String,
            display: Value,
            input_schema: Value,
        }

        let raw = RawDenToolDescriptor::deserialize(deserializer)?;
        let name: &'static str = Box::leak(raw.name.into_boxed_str());
        let provider_name = raw.provider_name;
        let provider_aliases: &'static [&'static str] = Box::leak(
            raw.provider_aliases
                .into_iter()
                .map(|item| Box::leak(item.into_boxed_str()) as &'static str)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let label: &'static str = Box::leak(raw.label.into_boxed_str());
        let description: &'static str = Box::leak(raw.description.into_boxed_str());
        let kind: &'static str = Box::leak(raw.kind.into_boxed_str());
        let provider: &'static str = Box::leak(raw.provider.into_boxed_str());
        let execution_target: &'static str = Box::leak(raw.execution_target.into_boxed_str());
        let scope: &'static str = Box::leak(raw.scope.into_boxed_str());
        let domain: &'static str = Box::leak(raw.domain.into_boxed_str());
        let content_class = raw
            .content_class
            .map(|value| Box::leak(value.into_boxed_str()) as &'static str);
        let availability: &'static str = Box::leak(raw.availability.into_boxed_str());
        let permissions: &'static [&'static str] = Box::leak(
            raw.permissions
                .into_iter()
                .map(|item| Box::leak(item.into_boxed_str()) as &'static str)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let allowed_roles: &'static [&'static str] = Box::leak(
            raw.allowed_roles
                .into_iter()
                .map(|item| Box::leak(item.into_boxed_str()) as &'static str)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let approval_policy: &'static str = Box::leak(raw.approval_policy.into_boxed_str());

        Ok(Self {
            name,
            provider_name,
            provider_aliases,
            label,
            description,
            kind,
            provider,
            execution_target,
            scope,
            domain,
            content_class,
            availability,
            permissions,
            allowed_roles,
            approval_policy,
            display: raw.display,
            input_schema: raw.input_schema,
        })
    }
}

#[cfg(test)]
mod test;
