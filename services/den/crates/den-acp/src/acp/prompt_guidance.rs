pub(super) fn maybe_workspace_tool_guidance(tool_names: &[&str]) -> Vec<String> {
    let mut guidance = Vec::new();
    if tool_names.contains(&"fs_list_directory") {
        guidance.push(
            "Use `fs_list_directory` with {{\"path\":\"/absolute/dir\",\"limit\":200}} to discover files.".to_string(),
        );
    }
    if tool_names.contains(&"fs_search_files") {
        guidance.push(
            "Use `fs_search_files` with {{\"path\":\"/absolute/path\",\"query\":\"text\",\"limit\":50,\"extensions\":[\"rs\"],\"pattern\":\"src/*\"}} to search.".to_string(),
        );
    }
    if tool_names.contains(&"fs_read_text_file") {
        guidance.push(
            "Use `fs_read_text_file` with {{\"path\":\"/absolute/file\",\"line\":1,\"limit\":400}} to read. Do not guess file contents.".to_string(),
        );
    }
    if tool_names.contains(&"fs_edit_file") {
        guidance.push(
            "Use `fs_edit_file` with {{\"path\":\"/absolute/file\",\"old_text\":\"exact\",\"new_text\":\"replacement\"}} to modify existing text files. It edits by replacing one exact `old_text` span with `new_text`, so read the file first and choose a unique span. Calling `fs_edit_file` is how you request local approval for an edit; do not ask for approval in chat when this tool is available.".to_string(),
        );
        guidance.push(
            "ACP edit workflow: discover/read the target, call `fs_edit_file` to request approval and perform the edit, wait for its result, verify the change with `fs_read_text_file`, then provide a concise final answer naming the changed file and what changed. Never claim you are blocked by approval if `fs_edit_file` is callable; invoke it instead.".to_string(),
        );
    } else {
        guidance.push(
            "No ACP edit tool is callable in this turn. Do not claim to request edit approval or ask for approval in chat; explain that editing is unavailable if asked to modify files.".to_string(),
        );
    }
    if tool_names.contains(&"fs_create_text_file") {
        guidance.push(
            "Use `fs_create_text_file` with {{\"path\":\"/absolute/new-file.txt\",\"content\":\"text\"}} to create new UTF-8 text files. It will not overwrite existing files; use `create_parent_dirs:true` only when parent directories should be created.".to_string(),
        );
    }
    if tool_names.contains(&"fs_delete_path") {
        guidance.push(
            "Use `fs_delete_path` with {{\"path\":\"/absolute/path\",\"expected_kind\":\"file\"}} to delete files or empty directories. For non-empty directories, `recursive:true` is required. Deleting workspace roots and sensitive paths is denied.".to_string(),
        );
    }
    guidance
}

pub(crate) fn server_memory_tool_guidance() -> Vec<String> {
    vec![
        "Use server tools for non-local capabilities: `session_info` for trusted information about the authenticated human, current bear, role, session, memory scopes, and policy; `memory_write_entry` only for durable pair-local semantic memory such as notes, logs, decisions, reflections, scratch, and summaries attributed to the authenticated human; `upsert_prompt_memory`, `patch_prompt_memory`, and `list_prompt_memory` for Den-owned editable runtime prompt memory blocks that should affect future prompt assembly; `memory_status`, `memory_browse`, `memory_read`, and `memory_search` to inspect Bear memory; `memory_request_review` to ask Reflection/curate to review role-local memory without writing shared memory directly; `update_task_list` to create and maintain the visible ACP session task list for the current work with at most one `in_progress` item; `get_task_list_status` and `list_task_lists` to recover visible task-list state; `request_task_list_handoff` when task-list work should become durable reviewed Docket work; `web_fetch` for bounded HTTP(S) page fetching; and `web_search` only when a Den search provider is configured. Do not switch ACP session modes yourself: Plan/Write/Ask mode is controlled by the user or ACP client UI. Prefer `update_task_list` whenever you need to remember 3 or more things to do, work has multiple steps, the user asks for a task list, or visible progress would help. If you are mentally tracking 3+ pending tasks, put them in the visible task list instead of keeping them only in prose. Each task-list item needs a `title` and `status`; Den auto-generates stable IDs when omitted, and exactly zero or one item should be `in_progress`. Treat task-list status as factual execution state: mark an item `completed` only after you actually performed the work or verified it was already done in this conversation/session; keep it `pending`, `in_progress`, or `blocked` if work remains. For completed non-trivial task-list items, include a concise summary of what was actually done or observed. Never mark tasks complete merely because you wrote a plan, summarized an intention, or described what should happen. Docket task `done` updates require a non-empty `result_summary`. Use concise prose alongside the visible task list. Do not use memory entry tools for active task lists, Docket tasks, observations, run results, Cabinet writes, or direct core updates.".to_string(),
        "Choose prompt memory tools when the user wants editable instruction-like runtime state that should shape future turns, such as role guidance, work-surface context, session focus, or explicit reusable user instructions. Choose `memory_write_entry` when the user wants durable recallable semantic memory rather than immediate prompt-shaping behavior.".to_string(),
        "Prompt memory is Den-owned runtime context, not semantic memory. Prefer `upsert_prompt_memory` to create or replace a block, `patch_prompt_memory` to change lifecycle/content for an existing block, and `list_prompt_memory` to inspect the current managed prompt-memory set. Use prompt-memory scopes deliberately: `session` for temporary session focus, `work_surface` for project/repo/service-specific guidance, `profile_local` for broader pair-role defaults, and `bear_wide` only for broad cross-surface guidance.".to_string(),
        "Memory is Bear-scoped across Workplaces and may contain multiple work surfaces. A Workplace is the role-scoped memory surface; for pair, that is the `pair` workplace. For questions about the current project, repo, service, architecture, terminology, or prior local decisions, first identify the relevant current work surface from trusted session hints, workspace roots, repo clues, or explicit user references rather than treating all Bear memory as one flat pool.".to_string(),
        "Prefer work-surface-first retrieval for local-understanding questions: current conversation and trusted session info -> current Workplace and current work-surface hints -> current work-surface canonical anchors -> current work-surface role-local working memory -> Bear-global shared anchors -> broader Bear memory search -> local workspace artifacts -> general world knowledge.".to_string(),
        "Use `memory_browse`, `memory_read`, and `memory_search` not only to recall prior notes, but to learn the current work surface within the current Workplace. If canonical work-surface anchors exist, prefer them over broad memory search for questions like 'what do you know about this?' or 'how does this work here?'.".to_string(),
        "Use `session_info.work_surface` as the trusted Den briefing for current Workplace/work-surface hints when available. Treat its reference candidates as guidance to resolve the active work surface, then confirm against canonical anchors and explicit user intent.".to_string(),
    ]
}

pub(super) fn tool_loop_rule_guidance() -> String {
    "Tool-loop rule: after any ACP tool result, continue from the returned content until the user's original request is complete. Do not stop merely because a tool succeeded. Do not ask the user whether to continue when the next step is implied by the original request. Stop only for required local approval, missing information, unrecoverable errors, or when you have verified and summarized completion. Never write textual tool-call syntax such as `to=functions...` or `functions.fs_edit_file`; if a tool is not callable, explain the limitation in normal prose.".to_string()
}
