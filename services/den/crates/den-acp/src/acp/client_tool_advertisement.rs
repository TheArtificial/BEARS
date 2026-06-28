//! ACP client tool advertisement (ADR-0043 step 5b stage B).
//!
//! The ACP-wire half of the client/edge tool catalog: the per-tool ACP wire
//! method names (`adapter_method` / `client_method`) and the JSON descriptor
//! advertisement sent to ACP clients at session start. The protocol-neutral tool
//! vocabulary (names, classes, policy, display) lives in
//! `den_core::client_tools`; this module frames it onto the ACP wire.

use serde_json::json;

use den_core::tools::tool_descriptor_guidance::{
    render_tool_descriptor_guidance, ToolDescriptorGuidance, ToolOrientationPolicy, ToolScopeKind,
    ToolSideEffectKind,
};
use den_core::client_tools::{
    client_tool_display, diag_phase, provider_tool_name_is_safe, ClientToolDescriptor,
    ClientToolName, ResolvedSessionPolicy, READ_TEXT_FILE_TOOL,
};

/// ACP wire method names for a client/edge tool. Owned by the ACP edge so the
/// protocol-neutral core descriptor carries no wire-method strings.
#[derive(Debug, Clone, Copy)]
pub struct AcpWire {
    pub adapter_method: &'static str,
    pub adapter_aliases: &'static [&'static str],
    pub client_method: &'static str,
    pub client_aliases: &'static [&'static str],
}

/// Map a client tool to its ACP adapter/client wire method names.
pub fn acp_wire(tool: ClientToolName) -> AcpWire {
    use ClientToolName as T;
    match tool {
        T::McpCallTool => AcpWire { adapter_method: "mcp/call_tool", adapter_aliases: &[], client_method: "mcp/call_tool", client_aliases: &[] },
        T::ReadTextFile => AcpWire { adapter_method: "bears/read_text_file", adapter_aliases: &[], client_method: "fs/read_text_file", client_aliases: &[] },
        T::ListDirectory => AcpWire { adapter_method: "bears/list_directory", adapter_aliases: &[], client_method: "fs/list_directory", client_aliases: &[] },
        T::FindPaths => AcpWire { adapter_method: "bears/find_paths", adapter_aliases: &[], client_method: "fs/find_paths", client_aliases: &[] },
        T::SearchFiles => AcpWire { adapter_method: "bears/search_files", adapter_aliases: &[], client_method: "fs/search_files", client_aliases: &[] },
        T::Stat => AcpWire { adapter_method: "bears/stat", adapter_aliases: &[], client_method: "fs/stat", client_aliases: &[] },
        T::EditFile => AcpWire { adapter_method: "bears/edit_file", adapter_aliases: &["bears/replace_text"], client_method: "fs/edit_file", client_aliases: &["fs/replace_text"] },
        T::CreateTextFile => AcpWire { adapter_method: "bears/create_text_file", adapter_aliases: &[], client_method: "fs/create_text_file", client_aliases: &[] },
        T::CreateDirectory => AcpWire { adapter_method: "bears/create_directory", adapter_aliases: &[], client_method: "fs/create_directory", client_aliases: &[] },
        T::MovePath => AcpWire { adapter_method: "bears/move_path", adapter_aliases: &[], client_method: "fs/move_path", client_aliases: &[] },
        T::CopyPath => AcpWire { adapter_method: "bears/copy_path", adapter_aliases: &[], client_method: "fs/copy_path", client_aliases: &[] },
        T::ApplyPatch => AcpWire { adapter_method: "bears/apply_patch", adapter_aliases: &[], client_method: "fs/apply_patch", client_aliases: &[] },
        T::DeletePath => AcpWire { adapter_method: "bears/delete_path", adapter_aliases: &[], client_method: "fs/delete_path", client_aliases: &[] },
        T::GitStatus => AcpWire { adapter_method: "bears/git_status", adapter_aliases: &[], client_method: "git/status", client_aliases: &[] },
        T::GitDiff => AcpWire { adapter_method: "bears/git_diff", adapter_aliases: &[], client_method: "git/diff", client_aliases: &[] },
        T::GitLog => AcpWire { adapter_method: "bears/git_log", adapter_aliases: &[], client_method: "git/log", client_aliases: &[] },
        T::GitShow => AcpWire { adapter_method: "bears/git_show", adapter_aliases: &[], client_method: "git/show", client_aliases: &[] },
        T::GitAdd => AcpWire { adapter_method: "bears/git_add", adapter_aliases: &[], client_method: "git/add", client_aliases: &[] },
        T::GitRestore => AcpWire { adapter_method: "bears/git_restore", adapter_aliases: &[], client_method: "git/restore", client_aliases: &[] },
        T::GitCommit => AcpWire { adapter_method: "bears/git_commit", adapter_aliases: &[], client_method: "git/commit", client_aliases: &[] },
        T::GitStash => AcpWire { adapter_method: "bears/git_stash", adapter_aliases: &[], client_method: "git/stash", client_aliases: &[] },
        T::ProcessRun => AcpWire { adapter_method: "bears/process_run", adapter_aliases: &[], client_method: "process/run", client_aliases: &[] },
        T::TerminalRunCommand => AcpWire { adapter_method: "bears/terminal_run_command", adapter_aliases: &[], client_method: "terminal/run_command", client_aliases: &[] },
        T::ChromeOpen => AcpWire { adapter_method: "bears/chrome_open", adapter_aliases: &[], client_method: "chrome/open", client_aliases: &[] },
        T::ChromeSnapshot => AcpWire { adapter_method: "bears/chrome_snapshot", adapter_aliases: &[], client_method: "chrome/snapshot", client_aliases: &[] },
        T::ChromeConsoleMessages => AcpWire { adapter_method: "bears/chrome_console_messages", adapter_aliases: &[], client_method: "chrome/console_messages", client_aliases: &[] },
        T::ChromeNetworkRequests => AcpWire { adapter_method: "bears/chrome_network_requests", adapter_aliases: &[], client_method: "chrome/network_requests", client_aliases: &[] },
        T::ChromeScreenshot => AcpWire { adapter_method: "bears/chrome_screenshot", adapter_aliases: &[], client_method: "chrome/screenshot", client_aliases: &[] },
    }
}

pub fn client_tool_descriptors() -> serde_json::Value {
    json!(ClientToolName::all()
        .iter()
        .map(|tool| client_tool_descriptor(*tool))
        .collect::<Vec<_>>())
}

pub fn client_tool_descriptors_for_client_context(
    client_context: &serde_json::Value,
    policy: Option<&ResolvedSessionPolicy>,
) -> serde_json::Value {
    // Compatibility rule: adapter-executed tools are advertised only when the
    // current adapter explicitly reports support. Adding a new local tool should
    // not force old adapters to update; they simply won't see the descriptor.
    let names = provider_tool_names_for_client_context(client_context, policy);
    let mut descriptors = names
        .iter()
        .filter_map(|name| ClientToolName::from_provider_alias(name))
        .map(client_tool_descriptor)
        .collect::<Vec<_>>();
    let mcp_tool_names = client_context
        .pointer("/mcp/client_tools")
        .and_then(|value| value.as_array())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.get("name").and_then(|value| value.as_str()))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(mcp_tools) = client_context
        .pointer("/mcp/client_tools")
        .and_then(|value| value.as_array())
    {
        // Zed forwards `context_servers` to external agents as ACP `mcpServers`.
        // The adapter connects to stdio MCP servers and publishes descriptors here.
        // A future MCP-over-ACP implementation should add a separate dynamic
        // descriptor source for `type: "acp"` servers once that draft RFD stabilizes.
        descriptors.extend(mcp_tools.iter().cloned());
    }
    tracing::info!(
        phase = diag_phase::DESCRIPTOR_ADVERTISED,
        static_tools = ?names,
        dynamic_mcp_tool_count = mcp_tool_names.len(),
        dynamic_mcp_tools = ?mcp_tool_names,
        final_descriptor_count = descriptors.len(),
        "ACP client tool descriptor assembly"
    );
    if names == vec![READ_TEXT_FILE_TOOL.provider_name]
        && !adapter_supports_tool(client_context, READ_TEXT_FILE_TOOL.provider_name)
    {
        tracing::info!(
            phase = diag_phase::DESCRIPTOR_ADVERTISED,
            tools = ?names,
            "ACP adapter did not advertise direct tools; falling back to read-text descriptor only"
        );
    } else {
        tracing::info!(
            phase = diag_phase::DESCRIPTOR_ADVERTISED,
            tools = ?names,
            "ACP client tool descriptors advertised"
        );
    }
    json!(descriptors)
}

pub fn provider_tool_names_for_client_context(
    client_context: &serde_json::Value,
    policy: Option<&ResolvedSessionPolicy>,
) -> Vec<&'static str> {
    let names = ClientToolName::all()
        .iter()
        .filter(|tool| **tool != ClientToolName::McpCallTool)
        .filter(|tool| adapter_supports_tool(client_context, tool.descriptor().provider_name))
        .filter(|tool| policy.is_none_or(|p| p.allows_tool(**tool)))
        .map(|tool| tool.descriptor().provider_name)
        .collect::<Vec<_>>();
    if names.is_empty() {
        vec![READ_TEXT_FILE_TOOL.provider_name]
    } else {
        names
    }
}

fn adapter_supports_tool(client_context: &serde_json::Value, provider_name: &str) -> bool {
    // Structured `adapter.direct_tools` is preferred. Legacy `direct_tools` is
    // accepted so older adapters keep working. New local tools must not bypass
    // this check.
    client_context
        .pointer(&format!("/adapter/direct_tools/{provider_name}/supported"))
        .and_then(|v| v.as_bool())
        .or_else(|| {
            client_context
                .pointer(&format!("/direct_tools/{provider_name}"))
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false)
}

pub fn read_text_file_client_tool_descriptor() -> serde_json::Value {
    client_tool_descriptor(ClientToolName::ReadTextFile)
}

fn chrome_descriptor(
    tool: &ClientToolDescriptor,
    wire: &AcpWire,
    properties: serde_json::Value,
    required: Vec<&str>,
) -> serde_json::Value {
    json!({
        "name": tool.provider_name,
        "description": format!(
            "ACP Chrome DevTools tool ({}, adapter={}, kind={}, risk={}). Requires BEARS_CHROME_CDP_URL or BEARS_BROWSER_CDP_URL pointing to a Chrome/Chromium/Edge CDP endpoint.",
            tool.canonical_name, wire.adapter_method, tool.kind, tool.risk,
        ),
        "parameters": {
            "type": "object",
            "properties": properties,
            "required": required,
        }
    })
}

fn acp_client_tool_domain(tool: &ClientToolDescriptor) -> &'static str {
    match tool.permission_class {
        "read_files" | "git_read" => "execution",
        "edit_files" | "delete_files" | "git_write" | "run_process" | "browser" => "execution",
        _ => "execution",
    }
}

fn acp_tool_guidance(tool: &ClientToolDescriptor) -> ToolDescriptorGuidance {
    match tool.permission_class {
        "read_files" => ToolDescriptorGuidance {
            scope: ToolScopeKind::AcpClientWorkspace,
            side_effect: ToolSideEffectKind::ReadOnly,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        },
        "edit_files" => ToolDescriptorGuidance {
            scope: ToolScopeKind::AcpClientWorkspace,
            side_effect: ToolSideEffectKind::WritesWorkspace,
            orientation: ToolOrientationPolicy::UseSessionInfoAndReadBeforeMutation,
        },
        "delete_files" => ToolDescriptorGuidance {
            scope: ToolScopeKind::AcpClientWorkspace,
            side_effect: ToolSideEffectKind::DeletesWorkspace,
            orientation: ToolOrientationPolicy::UseSessionInfoAndReadBeforeMutation,
        },
        "git_read" => ToolDescriptorGuidance {
            scope: ToolScopeKind::GitRepository,
            side_effect: ToolSideEffectKind::ReadOnly,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        },
        "git_write" => ToolDescriptorGuidance {
            scope: ToolScopeKind::GitRepository,
            side_effect: ToolSideEffectKind::GitMutation,
            orientation: ToolOrientationPolicy::UseSessionInfoAndInspectGitFirst,
        },
        "run_process" => ToolDescriptorGuidance {
            scope: ToolScopeKind::ProcessWorkspace,
            side_effect: ToolSideEffectKind::ExecutesCode,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        },
        "browser" => ToolDescriptorGuidance {
            scope: ToolScopeKind::BrowserSession,
            side_effect: ToolSideEffectKind::BrowserInteraction,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        },
        _ => ToolDescriptorGuidance {
            scope: ToolScopeKind::CurrentSession,
            side_effect: ToolSideEffectKind::ReadOnly,
            orientation: ToolOrientationPolicy::UseSessionInfoIfScopeUnclear,
        },
    }
}

fn append_scope_note(descriptor: &mut serde_json::Value, tool: &ClientToolDescriptor) {
    if let Some(description) = descriptor
        .as_object_mut()
        .and_then(|object| object.get_mut("description"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
    {
        descriptor["description"] = json!(format!(
            "{} {}",
            description,
            render_tool_descriptor_guidance(acp_tool_guidance(tool))
        ));
    }
}

pub fn client_tool_descriptor(tool: ClientToolName) -> serde_json::Value {
    let wire = acp_wire(tool);
    let tool = tool.descriptor();
    debug_assert!(provider_tool_name_is_safe(tool.provider_name));
    let mut descriptor = match tool.provider_name {
        "fs_read_text_file" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Reads a UTF-8 text file from the user's editor workspace through the local adapter. Use only for user workspace files, not server files.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute local file path under the workspace." },
                    "line": { "type": "integer", "minimum": 1, "description": "Optional 1-based starting line." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Optional maximum number of lines." }
                },
                "required": ["path"]
            }
        }),
        "fs_list_directory" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Lists entries in a workspace directory through the local adapter. Use this before reading files when you need to discover paths.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute local directory path under the workspace." },
                    "recursive": { "type": "boolean", "default": false, "description": "Whether to list recursively. Defaults to false." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "description": "Maximum entries to return." },
                    "include_hidden": { "type": "boolean", "default": false, "description": "Include hidden dotfiles and dot-directories. Defaults to false." }
                },
                "required": ["path"]
            }
        }),
        "fs_find_paths" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Finds workspace paths matching a glob pattern through the local adapter with bounded results.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "root": { "type": "string", "description": "Optional absolute directory path under the workspace. Defaults to the workspace root." },
                    "glob": { "type": "string", "description": "Glob pattern to match against relative paths, such as **/*.rs or package.json." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Maximum paths to return." },
                    "include_hidden": { "type": "boolean", "default": false, "description": "Include hidden dotfiles and dot-directories. Defaults to false." }
                },
                "required": ["glob"]
            }
        }),
        "fs_search_files" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Searches UTF-8 text files under a workspace path through the local adapter with bounded results and bytes. For filename/path discovery, set pattern (for example *notes*) and omit query or set query to an empty string.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute local file or directory path under the workspace." },
                    "query": { "type": "string", "description": "Optional literal text to search for inside files. If omitted or empty, pattern is used for filename/path discovery only." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Maximum matches to return." },
                    "max_bytes": { "type": "integer", "minimum": 1, "maximum": 1048576, "description": "Maximum total bytes to scan." },
                    "include_hidden": { "type": "boolean", "default": false, "description": "Include hidden dotfiles and dot-directories. Defaults to false." },
                    "case_sensitive": { "type": "boolean", "default": true, "description": "Whether literal matching is case-sensitive. Defaults to true." },
                    "pattern": { "type": "string", "description": "Optional simple wildcard pattern matched against relative file paths. Supports `*` and `?`." },
                    "extensions": { "type": "array", "items": { "type": "string" }, "maxItems": 10, "description": "Optional list of file extensions to include, such as [\"rs\", \"ts\"]." }
                },
                "required": ["path"]
            }
        }),
        "fs_stat" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Returns metadata for a workspace file or directory without reading file contents.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute local path under the workspace." },
                    "include_symlink_target": { "type": "boolean", "default": false, "description": "Include symlink target when the path is a symlink." }
                },
                "required": ["path"]
            }
        }),
        "fs_edit_file" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Edits an existing workspace text file by replacing one exact UTF-8 text span through the local adapter. Approval is required and sensitive paths are denied.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute local file path under the workspace." },
                    "old_text": { "type": "string", "description": "Exact text to replace. Must occur exactly once by default." },
                    "new_text": { "type": "string", "description": "Replacement text." },
                    "expected_replacements": { "type": "integer", "minimum": 1, "maximum": 1, "description": "Expected replacement count. Currently only 1 is allowed." },
                    "allow_multiple": { "type": "boolean", "default": false, "description": "Reserved for future use; currently must be false." },
                    "create_if_missing": { "type": "boolean", "default": false, "description": "Reserved for future use; currently must be false." }
                },
                "required": ["path", "old_text", "new_text"]
            }
        }),
        "fs_create_text_file" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Creates a new UTF-8 text file in the workspace through the local adapter. Approval is required; overwrite is disabled by default and sensitive paths are denied.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute local file path under the workspace." },
                    "content": { "type": "string", "description": "UTF-8 text content for the new file." },
                    "create_parent_dirs": { "type": "boolean", "default": false, "description": "Create parent directories if needed. Defaults to false." },
                    "overwrite": { "type": "boolean", "default": false, "description": "Reserved for future use; currently must be false." }
                },
                "required": ["path", "content"]
            }
        }),
        "fs_create_directory" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Creates a directory in the workspace through the local adapter. Approval is required; sensitive paths are denied by policy.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute local directory path under the workspace." },
                    "parents": { "type": "boolean", "default": false, "description": "Create parent directories if needed. Defaults to false." },
                    "allow_existing": { "type": "boolean", "default": false, "description": "Treat an existing directory as success. Defaults to false." }
                },
                "required": ["path"]
            }
        }),
        "fs_move_path" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Moves or renames a workspace file or directory through the local adapter. Approval is required; sensitive paths are denied by policy.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string", "description": "Absolute local source path under the workspace." },
                    "destination_path": { "type": "string", "description": "Absolute local destination path under the workspace." },
                    "overwrite": { "type": "boolean", "default": false, "description": "Overwrite destination when it already exists. Defaults to false." },
                    "expected_kind": { "type": "string", "enum": ["file", "directory", "any"], "description": "Optional expected source path kind. Defaults to any." }
                },
                "required": ["source_path", "destination_path"]
            }
        }),
        "fs_copy_path" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Copies a workspace file or directory through the local adapter. Approval is required; sensitive paths are denied by policy.",
                tool.canonical_name, "acp_client", wire.adapter_method, wire.client_method, tool.kind, tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string", "description": "Absolute local source path under the workspace." },
                    "destination_path": { "type": "string", "description": "Absolute local destination path under the workspace." },
                    "overwrite": { "type": "boolean", "default": false },
                    "recursive": { "type": "boolean", "default": false },
                    "expected_kind": { "type": "string", "enum": ["file", "directory", "any"] }
                },
                "required": ["source_path", "destination_path"]
            }
        }),
        "fs_apply_patch" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Applies a simple unified diff patch to workspace text files through the local adapter. This is not a fuzzy patch engine: provide full intended file content via context and added lines for each affected file. Approval is required; sensitive paths are denied by policy.",
                tool.canonical_name, "acp_client", wire.adapter_method, wire.client_method, tool.kind, tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "patch": { "type": "string", "description": "Unified diff patch." },
                    "base_path": { "type": "string", "description": "Optional absolute workspace directory path used to resolve relative patch paths." },
                    "dry_run": { "type": "boolean", "default": false },
                    "allow_create": { "type": "boolean", "default": true },
                    "allow_delete": { "type": "boolean", "default": false }
                },
                "required": ["patch"]
            }
        }),
        "fs_delete_path" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Deletes an existing workspace file or directory through the local adapter. Approval is required; sensitive paths and workspace roots are denied.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute local file or directory path under the workspace." },
                    "recursive": { "type": "boolean", "default": false, "description": "Required to delete non-empty directories." },
                    "expected_kind": { "type": "string", "enum": ["file", "directory", "any"], "description": "Optional expected path kind. Defaults to any." },
                    "allow_missing": { "type": "boolean", "default": false, "description": "If true, a missing path is treated as success." }
                },
                "required": ["path"]
            }
        }),
        "git_status" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Returns git status for a workspace repository through the local adapter.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "repo_path": { "type": "string", "description": "Optional absolute path under the workspace. Defaults to the workspace root." },
                    "include_untracked": { "type": "boolean", "default": true, "description": "Include untracked files. Defaults to true." }
                },
                "required": []
            }
        }),
        "git_diff" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Returns a bounded git diff for a workspace repository through the local adapter.",
                tool.canonical_name,
                "acp_client",
                wire.adapter_method,
                wire.client_method,
                tool.kind,
                tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "repo_path": { "type": "string", "description": "Optional absolute path under the workspace. Defaults to the workspace root." },
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Optional paths under the repository to limit the diff." },
                    "staged": { "type": "boolean", "default": false, "description": "Return staged diff instead of unstaged working-tree diff." },
                    "max_bytes": { "type": "integer", "minimum": 1, "maximum": 262144, "description": "Maximum diff bytes to return." }
                },
                "required": []
            }
        }),
        "git_log" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Returns a bounded git commit log for a workspace repository through the local adapter.",
                tool.canonical_name, "acp_client", wire.adapter_method, wire.client_method, tool.kind, tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "repo_path": { "type": "string", "description": "Optional absolute path under the workspace. Defaults to the workspace root." },
                    "max_count": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Maximum commits to return." },
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Optional paths under the repository to limit the log." }
                },
                "required": []
            }
        }),
        "git_show" => json!({
            "name": tool.provider_name,
            "description": format!(
                "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Shows a bounded git revision or file at revision for a workspace repository through the local adapter.",
                tool.canonical_name, "acp_client", wire.adapter_method, wire.client_method, tool.kind, tool.risk,
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "repo_path": { "type": "string", "description": "Optional absolute path under the workspace. Defaults to the workspace root." },
                    "revision": { "type": "string", "description": "Git revision, such as HEAD or a commit SHA." },
                    "path": { "type": "string", "description": "Optional path under the repository to show at the revision." },
                    "max_bytes": { "type": "integer", "minimum": 1, "maximum": 262144, "description": "Maximum output bytes to return." }
                },
                "required": ["revision"]
            }
        }),
        "git_add" => json!({
            "name": tool.provider_name,
            "description": "Stages explicit workspace repository paths with git add. Approval is required.",
            "parameters": { "type": "object", "properties": {
                "repo_path": { "type": "string" },
                "paths": { "type": "array", "items": { "type": "string" }, "minItems": 1 }
            }, "required": ["paths"] }
        }),
        "git_restore" => json!({
            "name": tool.provider_name,
            "description": "Restores explicit workspace repository paths with git restore. Approval is required because this can discard worktree or staged changes.",
            "parameters": { "type": "object", "properties": {
                "repo_path": { "type": "string" },
                "paths": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                "staged": { "type": "boolean", "default": false },
                "worktree": { "type": "boolean", "default": true },
                "source": { "type": "string" }
            }, "required": ["paths"] }
        }),
        "git_commit" => json!({
            "name": tool.provider_name,
            "description": "Creates a git commit from already staged changes. Approval is required.",
            "parameters": { "type": "object", "properties": {
                "repo_path": { "type": "string" },
                "message": { "type": "string" },
                "allow_empty": { "type": "boolean", "default": false }
            }, "required": ["message"] }
        }),
        "git_stash" => json!({
            "name": tool.provider_name,
            "description": "Creates a git stash for workspace repository changes. Approval is required.",
            "parameters": { "type": "object", "properties": {
                "repo_path": { "type": "string" },
                "message": { "type": "string" },
                "include_untracked": { "type": "boolean", "default": false }
            }, "required": [] }
        }),
        "process_run" | "terminal_run_command" => json!({
            "name": tool.provider_name,
            "description": if tool.provider_name == "terminal_run_command" {
                format!(
                    "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Runs an allowlisted build/test command in the client terminal and waits efficiently for terminal exit (useful when cargo waits for file locks). Approval is required.",
                    tool.canonical_name, "acp_client_terminal", wire.adapter_method, wire.client_method, tool.kind, tool.risk,
                )
            } else {
                format!(
                    "ACP local workspace tool ({}, target={}, adapter={}, client={}, kind={}, risk={}). Runs a bounded non-interactive process in an explicit workspace cwd through the local adapter. Approval is required.",
                    tool.canonical_name, "acp_client", wire.adapter_method, wire.client_method, tool.kind, tool.risk,
                )
            },
            "parameters": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": if tool.provider_name == "terminal_run_command" { "Allowlisted executable name, e.g. cargo, npm, pnpm, yarn, pytest, python3. Shell strings and paths are not accepted." } else { "Executable name or absolute executable path. Shell strings are not accepted." } },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Command arguments." },
                    "cwd": { "type": "string", "description": "Absolute working directory under the workspace." },
                    "timeout_ms": { "type": "integer", "minimum": 1, "maximum": if tool.provider_name == "terminal_run_command" { 600000 } else { 120000 } },
                    "max_output_bytes": { "type": "integer", "minimum": 1, "maximum": if tool.provider_name == "terminal_run_command" { 131072 } else { 65536 } },
                    "env": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Optional non-secret environment values." },
                    "reducer_mode": { "type": "string", "enum": ["execute_via_rtk", "postprocess", "none"], "default": "execute_via_rtk", "description": "Output reduction mode. Defaults to execute_via_rtk, which runs supported commands through RTK for compact output. Use none when the user asks for pure/noisy/raw command results." }
                },
                "required": ["command", "cwd"]
            }
        }),
        "chrome_open" => {
            chrome_descriptor(tool, &wire, json!({ "url": { "type": "string" } }), vec!["url"])
        }
        "chrome_snapshot" => chrome_descriptor(tool, &wire, json!({}), Vec::<&str>::new()),
        "chrome_console_messages" => chrome_descriptor(
            tool,
            &wire,
            json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 500 } }),
            Vec::<&str>::new(),
        ),
        "chrome_network_requests" => chrome_descriptor(
            tool,
            &wire,
            json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 500 } }),
            Vec::<&str>::new(),
        ),
        "chrome_screenshot" => chrome_descriptor(
            tool,
            &wire,
            json!({ "format": { "type": "string", "enum": ["png", "jpeg"] } }),
            Vec::<&str>::new(),
        ),
        _ => unreachable!("unknown ACP tool descriptor: {}", tool.provider_name),
    };
    append_scope_note(&mut descriptor, tool);
    if let Some(object) = descriptor.as_object_mut() {
        object.insert(
            "x-bears-domain".to_string(),
            json!(acp_client_tool_domain(tool)),
        );
        object.insert(
            "x-bears-content-class".to_string(),
            json!(tool.permission_class),
        );
        object.insert(
            "x-bears-display".to_string(),
            client_tool_display(
                ClientToolName::from_provider_alias(tool.provider_name)
                    .expect("descriptor provider name resolves"),
            )
            .to_json(),
        );
    }
    descriptor
}


#[cfg(test)]
mod tests {
    use super::*;
    use den_core::client_tools::*;

    // ADR-0043 s5b2: the ACP wire-method table now owns the adapter/client wire
    // names (removed from the protocol-neutral core descriptor) and feeds them into
    // the advertised descriptor.
    #[test]
    fn acp_wire_methods_feed_descriptor_advertisement() {
        // EditFile carries the only non-empty aliases, exercising the full shape.
        let edit = acp_wire(ClientToolName::EditFile);
        assert_eq!(edit.adapter_method, "bears/edit_file");
        assert_eq!(edit.client_method, "fs/edit_file");
        assert_eq!(edit.adapter_aliases, &["bears/replace_text"]);
        assert_eq!(edit.client_aliases, &["fs/replace_text"]);

        let advertised = client_tool_descriptor(ClientToolName::EditFile);
        let description = advertised["description"].as_str().unwrap_or("");
        assert!(description.contains("bears/edit_file"), "{description}");
        assert!(description.contains("fs/edit_file"), "{description}");

        for tool in ClientToolName::all() {
            let wire = acp_wire(*tool);
            assert!(!wire.adapter_method.is_empty(), "{tool:?}");
            assert!(!wire.client_method.is_empty(), "{tool:?}");
        }
    }

    #[test]
    fn provider_names_are_safe() {
        for tool in ClientToolName::all() {
            assert!(provider_tool_name_is_safe(tool.descriptor().provider_name));
        }
        assert!(!provider_tool_name_is_safe("fs.read_text_file"));
        assert!(!provider_tool_name_is_safe("fs/read_text_file"));
    }

    #[test]
    fn descriptors_use_provider_name_only() {
        let descriptors = client_tool_descriptors();
        let descriptors = descriptors.as_array().expect("descriptor array");
        assert_eq!(descriptors.len(), ClientToolName::all().len());
        for descriptor in descriptors {
            let name = descriptor["name"].as_str().expect("descriptor name");
            assert!(provider_tool_name_is_safe(name));
            let tool = ClientToolName::from_provider_alias(name).expect("known provider name");
            assert_eq!(name, tool.descriptor().provider_name);
            assert_ne!(name, tool.descriptor().canonical_name);
            assert_ne!(name, acp_wire(tool).client_method);
        }
        let serialized = serde_json::to_string(&descriptors).expect("serialize descriptors");
        assert!(!serialized.contains("\"name\":\"fs.read_text_file\""));
        assert!(!serialized.contains("\"name\":\"fs/read_text_file\""));
    }

    #[test]
    fn acp_local_tool_descriptors_include_scope_and_orientation_guidance() {
        for tool in ClientToolName::all() {
            let descriptor = client_tool_descriptor(*tool);
            let description = descriptor["description"].as_str().unwrap_or("");
            assert!(
                description.contains("Scope:"),
                "{} missing scope note: {}",
                tool.descriptor().provider_name,
                description
            );
            assert!(
                description.contains("session_info"),
                "{} missing session_info orientation note: {}",
                tool.descriptor().provider_name,
                description
            );
        }
    }

    #[test]
    fn descriptors_filter_by_adapter_direct_tools() {
        let descriptors = client_tool_descriptors_for_client_context(
            &json!({
                "direct_tools": {
                    "fs_read_text_file": true,
                    "fs_list_directory": true,
                    "fs_find_paths": true,
                    "fs_search_files": true,
                    "fs_stat": true,
                    "git_status": true,
                    "git_diff": true,
                    "fs_delete_path": true
                }
            }),
            None,
        );
        let names = descriptors
            .as_array()
            .expect("descriptor array")
            .iter()
            .map(|descriptor| descriptor["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(names.contains(&"fs_read_text_file"));
        assert!(names.contains(&"fs_list_directory"));
        assert!(names.contains(&"fs_find_paths"));
        assert!(names.contains(&"fs_search_files"));
        assert!(names.contains(&"fs_stat"));
        assert!(names.contains(&"git_status"));
        assert!(names.contains(&"git_diff"));
        assert!(names.contains(&"fs_delete_path"));
        assert!(!names.contains(&"fs_edit_file"));
    }

    #[test]
    fn descriptors_filter_by_structured_adapter_capabilities() {
        let descriptors = client_tool_descriptors_for_client_context(
            &json!({
                "adapter": {
                    "name": "bear-armature",
                    "version": "0.1.0",
                    "direct_tools": {
                        "fs_read_text_file": { "supported": true, "version": 1 },
                        "fs_find_paths": { "supported": true, "version": 1 },
                        "fs_stat": { "supported": true, "version": 1 },
                        "git_status": { "supported": true, "version": 1 },
                        "git_diff": { "supported": true, "version": 1 },
                        "git_log": { "supported": true, "version": 1 },
                        "git_show": { "supported": true, "version": 1 },
                        "fs_edit_file": { "supported": true, "version": 1 },
                        "fs_create_text_file": { "supported": true, "version": 1 },
                        "fs_create_directory": { "supported": true, "version": 1 },
                        "fs_move_path": { "supported": true, "version": 1 },
                        "fs_delete_path": { "supported": true, "version": 1 }
                    }
                }
            }),
            None,
        );
        let names = descriptors
            .as_array()
            .expect("descriptor array")
            .iter()
            .map(|descriptor| descriptor["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "fs_read_text_file",
                "fs_find_paths",
                "fs_stat",
                "fs_edit_file",
                "fs_create_text_file",
                "fs_create_directory",
                "fs_move_path",
                "fs_delete_path",
                "git_status",
                "git_diff",
                "git_log",
                "git_show"
            ]
        );
    }

    #[test]
    fn missing_direct_tools_defaults_to_read_text_only() {
        let descriptors = client_tool_descriptors_for_client_context(&json!({}), None);
        let names = descriptors
            .as_array()
            .expect("descriptor array")
            .iter()
            .map(|descriptor| descriptor["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["fs_read_text_file"]);
    }

    #[test]
    fn resolve_session_policy_defaults_to_ask_with_read_only_enablement() {
        let policy = resolve_session_policy(None);
        assert_eq!(policy.mode_label, "Ask");
        assert_eq!(policy.tool_enablement.as_str(), "read_only");
        assert_eq!(policy.allowed_tool_classes(), vec!["read_only"]);
        assert_eq!(
            policy.denied_tool_classes(),
            vec!["workspace_mutation", "execution", "browser"]
        );
        assert!(policy.allows_tool(ClientToolName::ReadTextFile));
        assert!(!policy.allows_tool(ClientToolName::EditFile));
        assert!(!policy.allows_tool(ClientToolName::ProcessRun));
        assert!(!policy.allows_tool(ClientToolName::ChromeOpen));
    }

    #[test]
    fn resolve_session_policy_marks_plan_as_read_only_mode() {
        let policy = resolve_session_policy_for_mode("ask", Some("active"));
        assert_eq!(policy.mode_label, "Plan");
        assert_eq!(policy.tool_enablement.as_str(), "read_only");
        assert_eq!(policy.plan_mode_state.as_deref(), Some("active"));
        assert!(policy.allows_tool(ClientToolName::GitStatus));
        assert!(!policy.allows_tool(ClientToolName::EditFile));
        assert!(!policy.allows_tool(ClientToolName::ProcessRun));
        assert!(!policy.allows_tool(ClientToolName::ChromeOpen));
    }

    #[test]
    fn resolve_session_policy_marks_write_with_all_tools_enabled() {
        let policy = resolve_session_policy_for_mode("ask", Some("approved"));
        assert_eq!(policy.mode_label, "Write");
        assert_eq!(policy.tool_enablement.as_str(), "all_tools");
        assert_eq!(policy.plan_mode_state.as_deref(), Some("approved"));
        assert_eq!(
            policy.allowed_tool_classes(),
            vec!["read_only", "workspace_mutation", "execution", "browser"]
        );
        assert!(policy.denied_tool_classes().is_empty());
        assert!(policy.allows_tool(ClientToolName::EditFile));
        assert!(policy.allows_tool(ClientToolName::ProcessRun));
        assert!(policy.allows_tool(ClientToolName::ChromeOpen));
    }

    #[test]
    fn descriptor_filtering_keeps_only_read_tools_available_in_ask_mode() {
        let policy = resolve_session_policy(None);
        let descriptors = client_tool_descriptors_for_client_context(
            &json!({
                "adapter": {
                    "direct_tools": {
                        "fs_read_text_file": { "supported": true },
                        "fs_edit_file": { "supported": true },
                        "process_run": { "supported": true },
                        "chrome_open": { "supported": true }
                    }
                }
            }),
            Some(&policy),
        );
        let names = descriptors
            .as_array()
            .expect("descriptor array")
            .iter()
            .map(|descriptor| descriptor["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["fs_read_text_file"]);
    }

    #[test]
    fn provider_name_filtering_respects_write_tool_enablement() {
        let policy = resolve_session_policy_for_mode("ask", Some("approved"));
        let names = provider_tool_names_for_client_context(
            &json!({
                "adapter": {
                    "direct_tools": {
                        "fs_read_text_file": { "supported": true },
                        "fs_edit_file": { "supported": true },
                        "process_run": { "supported": true },
                        "chrome_open": { "supported": true }
                    }
                }
            }),
            Some(&policy),
        );
        assert_eq!(
            names,
            vec![
                "fs_read_text_file",
                "fs_edit_file",
                "process_run",
                "chrome_open"
            ]
        );
    }

    #[test]
    fn read_text_file_descriptor_wrapper_still_works() {
        let descriptor = read_text_file_client_tool_descriptor();
        assert_eq!(descriptor["name"], READ_TEXT_FILE_TOOL.provider_name);
    }

    #[test]
    fn tool_policy_includes_authoritative_limits_and_scope() {
        let list_policy = client_tool_policy_json_for_provider("fs_list_directory");
        assert_eq!(list_policy["scope_basis"], "acp:tools");
        assert_eq!(list_policy["role_basis"], "pair_agent");
        assert_eq!(
            list_policy["allowed_roots_basis"],
            "acp_session.workspace_roots"
        );
        assert_eq!(list_policy["max_entries"], 1000);
        assert_eq!(list_policy["include_hidden_default"], false);

        let find_policy = client_tool_policy_json_for_provider("fs_find_paths");
        assert_eq!(find_policy["max_results"], 500);
        assert_eq!(find_policy["include_hidden_default"], false);

        let search_policy = client_tool_policy_json_for_provider("fs_search_files");
        assert_eq!(search_policy["max_results"], 200);
        assert_eq!(search_policy["max_bytes"], 1_048_576);
        assert_eq!(search_policy["approval_required"], true);

        let stat_policy = client_tool_policy_json_for_provider("fs_stat");
        assert_eq!(stat_policy["risk"], "read_only");
        assert_eq!(stat_policy["approval_required"], true);

        let git_status_policy = client_tool_policy_json_for_provider("git_status");
        assert_eq!(git_status_policy["risk"], "read_only");
        assert_eq!(git_status_policy["max_results"], 500);
        assert_eq!(git_status_policy["max_bytes"], 262_144);

        let git_diff_policy = client_tool_policy_json_for_provider("git_diff");
        assert_eq!(git_diff_policy["risk"], "read_only");
        assert_eq!(git_diff_policy["max_bytes"], 262_144);

        let git_log_policy = client_tool_policy_json_for_provider("git_log");
        assert_eq!(git_log_policy["risk"], "read_only");
        assert_eq!(git_log_policy["max_results"], 100);
        assert_eq!(git_log_policy["max_bytes"], 262_144);

        let git_show_policy = client_tool_policy_json_for_provider("git_show");
        assert_eq!(git_show_policy["risk"], "read_only");
        assert_eq!(git_show_policy["max_bytes"], 262_144);

        let replace_policy = client_tool_policy_json_for_provider("fs_edit_file");
        assert_eq!(replace_policy["risk"], "writes_workspace");
        assert_eq!(
            replace_policy["sensitive_path_policy"],
            "deny_sensitive_paths"
        );
        assert_eq!(replace_policy["max_replacements"], 1);
        assert_eq!(replace_policy["create_files"], false);
        assert_eq!(
            replace_policy["allow_multiple"],
            serde_json::Value::Bool(false)
        );
        assert_eq!(replace_policy["deny_hidden_paths"], true);
        assert!(replace_policy.get("max_results").is_none());
        assert_eq!(replace_policy["approval_required"], true);

        let create_policy = client_tool_policy_json_for_provider("fs_create_text_file");
        assert_eq!(create_policy["risk"], "writes_workspace");
        assert_eq!(create_policy["create_files"], true);
        assert_eq!(create_policy["max_bytes"], 1_048_576);

        let create_directory_policy = client_tool_policy_json_for_provider("fs_create_directory");
        assert_eq!(create_directory_policy["risk"], "writes_workspace");
        assert_eq!(create_directory_policy["create_files"], true);
        assert_eq!(create_directory_policy["deny_hidden_paths"], true);

        let move_policy = client_tool_policy_json_for_provider("fs_move_path");
        assert_eq!(move_policy["risk"], "writes_workspace");
        assert_eq!(move_policy["deny_hidden_paths"], true);

        let delete_policy = client_tool_policy_json_for_provider("fs_delete_path");
        assert_eq!(delete_policy["risk"], "deletes_workspace");
        assert_eq!(
            delete_policy["sensitive_path_policy"],
            "deny_sensitive_paths"
        );
        assert_eq!(delete_policy["max_entries"], 100);
        assert_eq!(delete_policy["deny_hidden_paths"], true);
    }

    #[test]
    fn all_advertised_tools_require_approval_and_adapter_path_containment() {
        for tool in ClientToolName::all() {
            let descriptor = tool.descriptor();
            let policy = client_tool_policy(*tool).to_json(descriptor);
            assert_eq!(
                policy["approval_required"], true,
                "{}",
                descriptor.provider_name
            );
            assert!(
                matches!(
                    policy["path_containment"].as_str(),
                    Some(
                        "adapter_enforced_absolute_path_under_allowed_roots"
                            | "adapter_enforced_absolute_cwd_under_allowed_roots"
                            | "adapter_enforced_url_host_scope"
                            | "adapter_enforced_chrome_cdp_endpoint"
                    )
                ),
                "{}",
                descriptor.provider_name
            );
            assert!(
                matches!(
                    policy["allowed_roots_basis"].as_str(),
                    Some("acp_session.workspace_roots" | "url.host" | "chrome_cdp_endpoint")
                ),
                "{}",
                descriptor.provider_name
            );
            assert!(
                policy["permission_timeout_ms"].as_u64().unwrap()
                    <= policy["total_timeout_ms"].as_u64().unwrap(),
                "permission timeout must fit inside tool timeout for {}",
                descriptor.provider_name
            );
        }
    }

    #[test]
    fn mutating_tools_deny_sensitive_paths_and_allow_hidden_paths_by_policy() {
        for name in [
            "fs_edit_file",
            "fs_create_text_file",
            "fs_create_directory",
            "fs_move_path",
            "fs_delete_path",
            "fs_copy_path",
            "fs_apply_patch",
        ] {
            let policy = client_tool_policy_json_for_provider(name);
            assert_eq!(
                policy["sensitive_path_policy"], "deny_sensitive_paths",
                "{name}"
            );
            assert_eq!(policy["deny_hidden_paths"], true, "{name}");
            assert!(
                matches!(
                    policy["risk"].as_str(),
                    Some("writes_workspace" | "deletes_workspace")
                ),
                "{name} must have mutating risk"
            );
        }
    }

    #[test]
    fn milestone_1_descriptor_schemas_are_present() {
        let find = client_tool_descriptor(ClientToolName::FindPaths);
        assert_eq!(find["parameters"]["required"], json!(["glob"]));
        assert!(find["parameters"]["properties"].get("root").is_some());
        assert!(find["parameters"]["properties"]
            .get("include_hidden")
            .is_some());

        let stat = client_tool_descriptor(ClientToolName::Stat);
        assert_eq!(stat["parameters"]["required"], json!(["path"]));
        assert!(stat["parameters"]["properties"]
            .get("include_symlink_target")
            .is_some());

        let create_directory = client_tool_descriptor(ClientToolName::CreateDirectory);
        assert_eq!(create_directory["parameters"]["required"], json!(["path"]));
        assert!(create_directory["parameters"]["properties"]
            .get("parents")
            .is_some());
        assert!(create_directory["parameters"]["properties"]
            .get("allow_existing")
            .is_some());

        let move_path = client_tool_descriptor(ClientToolName::MovePath);
        assert_eq!(
            move_path["parameters"]["required"],
            json!(["source_path", "destination_path"])
        );
        assert!(move_path["parameters"]["properties"]
            .get("overwrite")
            .is_some());
        assert!(move_path["parameters"]["properties"]
            .get("expected_kind")
            .is_some());

        let git_status = client_tool_descriptor(ClientToolName::GitStatus);
        assert_eq!(git_status["parameters"]["required"], json!([]));
        assert!(git_status["parameters"]["properties"]
            .get("repo_path")
            .is_some());

        let git_diff = client_tool_descriptor(ClientToolName::GitDiff);
        assert_eq!(git_diff["parameters"]["required"], json!([]));
        assert!(git_diff["parameters"]["properties"].get("paths").is_some());
        assert!(git_diff["parameters"]["properties"].get("staged").is_some());

        let git_log = client_tool_descriptor(ClientToolName::GitLog);
        assert_eq!(git_log["parameters"]["required"], json!([]));
        assert!(git_log["parameters"]["properties"]
            .get("max_count")
            .is_some());
        assert!(git_log["parameters"]["properties"].get("paths").is_some());

        let git_show = client_tool_descriptor(ClientToolName::GitShow);
        assert_eq!(git_show["parameters"]["required"], json!(["revision"]));
        assert!(git_show["parameters"]["properties"].get("path").is_some());
        assert!(git_show["parameters"]["properties"]
            .get("max_bytes")
            .is_some());
    }

    #[test]
    fn descriptor_schemas_keep_search_query_optional_for_path_discovery() {
        let descriptor = client_tool_descriptor(ClientToolName::SearchFiles);
        let required = descriptor["parameters"]["required"].as_array().unwrap();
        assert_eq!(required, &vec![json!("path")]);
        assert!(descriptor["parameters"]["properties"]
            .get("pattern")
            .is_some());
        assert!(descriptor["parameters"]["properties"]
            .get("extensions")
            .is_some());
        assert!(descriptor["parameters"]["properties"]
            .get("case_sensitive")
            .is_some());
    }
}
