
use super::*;

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
