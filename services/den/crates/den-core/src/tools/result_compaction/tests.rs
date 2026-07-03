use super::*;

#[test]
fn compact_client_tool_result_truncates_large_content_for_model() {
    let long = "x".repeat(40 * 1024);
    let input = ClientToolResultInput::new(
        "call_large",
        None,
        ToolResultStatus::Ok,
        Some(long),
        json!({ "nested": "y".repeat(40 * 1024) }),
        Value::Null,
    );
    let compacted = compact_client_tool_result(&input);

    assert_eq!(compacted.payload["tool_call_id"], "call_large");
    assert_eq!(compacted.payload["result_compaction"]["truncated"], true);
    assert!(compacted
        .content
        .contains("tool result truncated for model context"));
    assert!(compacted.content.chars().count() < 25 * 1024);
}

#[test]
fn compact_client_tool_result_preserves_tool_name_for_projection() {
    let input = ClientToolResultInput::new(
        "call_named",
        Some("fs_read_text_file".to_string()),
        ToolResultStatus::Ok,
        Some("".to_string()),
        json!({ "content": "hello" }),
        Value::Null,
    );
    let compacted = compact_client_tool_result(&input);

    assert_eq!(compacted.payload["tool_name"], "fs_read_text_file");
    assert!(compacted.payload["error"].is_null());
    assert_eq!(
        compacted.payload["output_summary"],
        "Used fs_read_text_file (ok): hello"
    );
    assert_eq!(compacted.payload["output_preview"], "hello");
}

#[test]
fn compact_client_tool_result_adds_bounded_summary_without_preview() {
    let input = ClientToolResultInput::new(
        "call_status_only",
        Some("session_info".to_string()),
        ToolResultStatus::Ok,
        Some("".to_string()),
        Value::Null,
        Value::Null,
    );
    let compacted = compact_client_tool_result(&input);

    assert_eq!(compacted.payload["output_summary"], "Used session_info (ok)");
    assert!(compacted.payload.get("output_preview").is_none());
}

#[test]
fn compact_client_tool_result_falls_back_to_structured_content_when_content_is_empty() {
    let input = ClientToolResultInput::new(
        "call_read_file",
        Some("fs_read_text_file".to_string()),
        ToolResultStatus::Ok,
        Some("".to_string()),
        json!({ "content": "hello from file" }),
        Value::Null,
    );
    let compacted = compact_client_tool_result(&input);

    assert_eq!(compacted.content, "hello from file");
    assert_eq!(
        compacted.payload["output_summary"],
        "Used fs_read_text_file (ok): hello from file"
    );
    assert_eq!(compacted.payload["output_preview"], "hello from file");
}

#[test]
fn compact_json_tool_result_truncates_large_den_hosted_result() {
    let compacted = compact_json_tool_result(json!({
        "results": [{ "body": "x".repeat(40 * 1024) }]
    }));

    assert_eq!(compacted.payload["result_compaction"]["truncated"], true);
    assert!(compacted
        .content
        .contains("tool result truncated for model context"));
    assert!(compacted.content.chars().count() < 25 * 1024);
}
