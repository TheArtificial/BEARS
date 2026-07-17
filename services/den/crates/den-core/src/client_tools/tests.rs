use super::*;
use crate::tools::descriptor::builtin_den_tool_descriptors;

#[test]
fn all_client_tool_policies_have_descriptor_owned_execution_contract() {
    for tool in ClientToolName::all() {
        let descriptor = tool.descriptor();
        let policy = client_tool_policy(*tool);
        assert_eq!(
            policy.execution_target,
            ExecutionTargetPolicy::ArmatureLocal,
            "client tool {tool:?} must be armature-owned"
        );
        let json = policy.to_json(descriptor);
        assert_eq!(json["execution_target"], "armature_local");
        assert!(
            json.get("approval_policy").is_some(),
            "{tool:?} missing approval_policy"
        );
        assert!(
            json.get("target_policy").is_some(),
            "{tool:?} missing target_policy"
        );
        assert_eq!(json["provider_tool"], descriptor.provider_name);
    }
}

#[test]
fn all_den_tool_descriptors_have_supported_execution_target() {
    for descriptor in builtin_den_tool_descriptors() {
        assert_eq!(
            descriptor.execution_target, "den",
            "Den tool {} must be owned by Den or explicitly moved to a typed non-Den descriptor",
            descriptor.provider_name
        );
    }
}

#[test]
fn provider_tool_descriptor_read_file_requires_path() {
    let descriptor = provider_tool_descriptor(ClientToolName::ReadTextFile);
    assert_eq!(descriptor["name"], "fs_read_text_file");
    assert_eq!(descriptor["parameters"]["required"], json!(["path"]));
    assert_eq!(descriptor["parameters"]["additionalProperties"], false);
    assert!(descriptor["parameters"]["properties"].get("path").is_some());
    assert!(descriptor["parameters"]["properties"].get("line").is_some());
    assert!(descriptor["parameters"]["properties"]
        .get("limit")
        .is_some());
}

#[test]
fn read_only_fs_tool_policies_do_not_require_permission() {
    for tool in [
        ClientToolName::ReadTextFile,
        ClientToolName::ListDirectory,
        ClientToolName::FindPaths,
        ClientToolName::SearchFiles,
        ClientToolName::Stat,
    ] {
        let policy = client_tool_policy(tool);
        assert_eq!(
            policy.approval_policy,
            ApprovalPolicy::Never,
            "{tool:?} should not pause for permission"
        );
        assert!(!policy.approval_policy.requires_unconditional_approval());
    }
}

#[test]
fn submitted_plan_keeps_write_tools_locked() {
    let policy = resolve_session_policy_for_mode("write", Some("submitted"));

    assert_eq!(policy.mode_label, "Plan");
    assert_eq!(policy.tool_enablement, ToolEnablementState::ReadOnly);
    assert!(!policy.allows_tool(ClientToolName::EditFile));
    assert!(policy.allows_tool(ClientToolName::ReadTextFile));
    assert_eq!(
        policy.denied_tool_classes(),
        vec!["workspace_mutation", "execution", "browser"]
    );
}

#[test]
fn turn_authority_is_single_derived_permission_surface() {
    let authority = TurnAuthority::for_session_mode(
        BearStance::Pair,
        Governance::Interactive,
        "write",
        Some("submitted"),
    );

    assert_eq!(authority.mode_label(), "Plan");
    assert_eq!(authority.tool_enablement(), ToolEnablementState::ReadOnly);
    assert!(authority.allows_tool(ClientToolName::ReadTextFile));
    assert!(!authority.allows_tool(ClientToolName::EditFile));
    assert_eq!(authority.allowed_tool_classes(), vec!["read_only"]);
    assert_eq!(
        authority.denied_tool_classes(),
        vec!["workspace_mutation", "execution", "browser"]
    );
    assert!(authority
        .read_only_runtime_context()
        .expect("read-only context")
        .contains("permission_mode=`Plan`; tool_enablement=`read_only`"));
}

#[test]
fn find_paths_policy_is_descriptor_owned() {
    let policy = client_tool_policy(ClientToolName::FindPaths);
    assert_eq!(
        policy.execution_target,
        ExecutionTargetPolicy::ArmatureLocal
    );
    assert_eq!(policy.approval_policy, ApprovalPolicy::Never);
    assert_eq!(
        policy.sensitive_path_policy,
        SensitivePathPolicy::FilterResults
    );
    assert_eq!(
        policy.target_policy,
        TargetPolicy::WorkspaceRootOrPath {
            arg: Some("root"),
            default_to_workspace_root: true,
            required_kind: Some(FsTargetKindPolicy::Directory),
        }
    );
    let json = policy.to_json(&FIND_PATHS_TOOL);
    assert_eq!(json["approval_policy"], "never");
    assert_eq!(json["approval_required"], false);
    assert_eq!(json["sensitive_path_policy"], "filter_results");
    assert_eq!(json["target_policy"]["kind"], "workspace_root_or_path");
}

#[test]
fn provider_tool_descriptor_find_paths_requires_glob() {
    let descriptor = provider_tool_descriptor(ClientToolName::FindPaths);
    assert_eq!(descriptor["name"], "fs_find_paths");
    assert_eq!(descriptor["parameters"]["required"], json!(["glob"]));
    assert_eq!(descriptor["parameters"]["additionalProperties"], false);
    assert!(descriptor["parameters"]["properties"].get("glob").is_some());
    assert!(descriptor["parameters"]["properties"].get("root").is_some());
    assert!(descriptor["parameters"]["properties"]
        .get("include_hidden")
        .is_some());
}

#[test]
fn provider_tool_descriptor_run_command_has_command_schema() {
    let descriptor = provider_tool_descriptor(ClientToolName::RunCommand);
    assert_eq!(descriptor["name"], "run_command");
    assert_eq!(descriptor["parameters"]["required"], json!(["command"]));
    assert_eq!(descriptor["parameters"]["additionalProperties"], false);
    assert!(descriptor["parameters"]["properties"]
        .get("command")
        .is_some());
    assert!(descriptor["parameters"]["properties"].get("args").is_some());
    assert!(descriptor["parameters"]["properties"].get("cwd").is_some());
    assert!(descriptor["parameters"]["properties"]
        .get("bypass_tool_redirect")
        .is_some());
}

#[test]
fn client_tool_display_includes_run_command_args() {
    let display = client_tool_display_for_provider(
        "run_command",
        &json!({ "command": "git", "args": ["status", "--short"], "cwd": "/workspace" }),
    );

    assert_eq!(
        display["title"],
        "Running command git status --short → …/workspace"
    );
    assert_eq!(display["subtitle"], "git status --short → …/workspace");
}

#[test]
fn client_tool_display_uses_short_display_paths_for_workspace_targets() {
    let display = client_tool_display_for_provider(
        "fs_read_text_file",
        &json!({ "path": "/workspace/project/src/main.rs" }),
    );
    assert_eq!(display["title"], "Reading …/project/src/main.rs");
    assert_eq!(display["subtitle"], "…/project/src/main.rs");
    assert_eq!(display["target"]["path"], "…/project/src/main.rs");

    let move_display = client_tool_display_for_provider(
        "fs_move_path",
        &json!({
            "source_path": "/workspace/project/src/main.rs",
            "destination_path": "/workspace/project/src/lib.rs"
        }),
    );
    assert_eq!(
        move_display["title"],
        "Moving …/project/src/main.rs → …/project/src/lib.rs"
    );
}
