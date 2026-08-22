use super::*;
use futures::StreamExt;

fn noop_pg_pool() -> PgPool {
    PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").expect("lazy pg pool")
}

async fn collect_bear_channel_proxy_output(frames: Vec<&'static str>) -> String {
    let upstream = futures::stream::iter(
        frames
            .into_iter()
            .map(|frame| Ok::<Bytes, reqwest::Error>(Bytes::from_static(frame.as_bytes()))),
    );
    let mut stream = BearChannelSseProxyStream::new(
        upstream,
        Uuid::parse_str("f42114ea-99bd-48a7-818a-78d4e3d914be").unwrap(),
        1,
        Uuid::parse_str("b4b3413e-2e5c-4230-baf0-53bfe4725d4c").unwrap(),
        "test-conversation".to_string(),
        noop_pg_pool(),
    );
    let mut out = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("proxy chunk");
        out.push_str(std::str::from_utf8(&chunk).expect("utf8"));
    }
    out
}

fn mapped_text(frame: &str) -> String {
    let mut out = String::new();
    for line in frame.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
            if let Some(bytes) = bear_channel_event_to_deep_chat_sse(&value) {
                out.push_str(std::str::from_utf8(&bytes).expect("utf8"));
            }
        }
    }
    out
}

#[test]
fn deep_chat_sse_body_for_assistant_text_maps_to_assistant_message() {
    let body = deep_chat_sse_body_for_assistant_text("Capabilities list");
    assert!(body.contains("\"message_type\":\"assistant_message\""));
    assert!(body.contains("\"content\":\"Capabilities list\""));
    assert!(body.starts_with("data: "));
}

#[test]
fn maps_assistant_delta_to_deep_chat_sse() {
    let out = mapped_text("data: {\"type\":\"assistant_delta\",\"text\":\"Hi\",\"id\":\"a1\"}\n\n");
    assert!(out.starts_with("data: "));
    assert!(out.contains("\"message_type\":\"assistant_message\""));
    assert!(out.contains("\"content\":\"Hi\""));
}

#[test]
fn maps_direct_assistant_message_to_deep_chat_sse() {
    let out = mapped_text(
        "data: {\"message_type\":\"assistant_message\",\"content\":\"Hi\",\"id\":\"a1\"}\n\n",
    );
    assert!(out.starts_with("data: "));
    assert!(out.contains("\"message_type\":\"assistant_message\""));
    assert!(out.contains("\"content\":\"Hi\""));
}

#[test]
fn maps_wrapped_assistant_message_to_deep_chat_sse() {
    let out = mapped_text(
            "data: {\"contents\":{\"message_type\":\"assistant_message\",\"content\":[{\"text\":\"Hi\"},{\"text\":\" there\"}]}}\n\n",
        );
    assert!(out.starts_with("data: "));
    assert!(out.contains("\"message_type\":\"assistant_message\""));
    assert!(out.contains("\"content\":\"Hi there\""));
}

#[test]
fn direct_assistant_message_counts_as_substantive_reply() {
    let mut persistence = PendingConversationPersistence::default();
    persistence.ingest(&serde_json::json!({
        "message_type": "assistant_message",
        "content": "Rendered reply"
    }));
    assert!(persistence_has_substantive_reply(&persistence));
    assert_eq!(persistence.assistant_text, "Rendered reply");
}

#[test]
fn wrapped_assistant_message_counts_as_substantive_reply() {
    let mut persistence = PendingConversationPersistence::default();
    persistence.ingest(&serde_json::json!({
        "contents": {
            "message_type": "assistant_message",
            "content": [{"text": "Rendered"}, {"text": " reply"}]
        }
    }));
    assert!(persistence_has_substantive_reply(&persistence));
    assert_eq!(persistence.assistant_text, "Rendered reply");
}

#[test]
fn maps_status_progress_to_status_message() {
    let out = mapped_text("data: {\"type\":\"status_progress\",\"text\":\"Thinking…\"}\n\n");
    assert!(out.contains("\"message_type\":\"status_message\""));
    assert!(out.contains("Thinking…"));
    assert!(!out.contains("reasoning_message"));
}

#[test]
fn persistence_ingest_ignores_status_progress() {
    let mut persistence = PendingConversationPersistence::default();
    persistence.ingest(&serde_json::json!({"type": "status_progress", "text": "Thinking…"}));
    assert!(!persistence.has_flushable_content());
}

#[test]
fn strip_ephemeral_status_suffixes_removes_trailing_pollution() {
    assert_eq!(strip_ephemeral_status_suffixes("HelloThinking…"), "Hello");
    assert_eq!(strip_ephemeral_status_suffixes("Thinking…"), "");
}

#[test]
fn maps_reasoning_delta_to_deep_chat_sse() {
    let out = mapped_text("data: {\"type\":\"reasoning_delta\",\"text\":\"Thinking\"}\n\n");
    assert!(out.contains("\"message_type\":\"reasoning_message\""));
    assert!(out.contains("\"reasoning\":\"Thinking\""));
}

#[test]
fn persistence_ingest_tracks_browser_visible_content() {
    let mut persistence = PendingConversationPersistence::default();
    assert!(!persistence.has_flushable_content());
    persistence.ingest(&serde_json::json!({"type": "assistant_delta", "text": "Hello"}));
    assert!(persistence.has_flushable_content());
    persistence.ingest(&serde_json::json!({"type": "error", "message": "Tool failed"}));
    persistence.ingest(&serde_json::json!({
        "type": "server_tool_started",
        "tool": "memory_read",
        "summary": "memory_read"
    }));
    persistence.ingest(&serde_json::json!({
        "type": "subagent_finished",
        "name": "researcher",
        "summary": "done"
    }));
    assert!(!persistence.assistant_text.is_empty());
    assert!(!persistence.error_text.is_empty());
    assert!(!persistence.status_text.is_empty());
}

#[test]
fn maps_error_to_deep_chat_sse() {
    let out = mapped_text("data: {\"type\":\"error\",\"message\":\"Nope\",\"detail\":\"More\",\"request_id\":\"r1\",\"context\":{\"upstream_error\":[{\"param\":\"tools[15].name\"}]}}\n\n");
    assert!(out.contains("\"message_type\":\"error_message\""));
    assert!(out.contains("\"message\":\"Nope\""));
    assert!(out.contains("\"support_ref\":\"r1\""));
    assert!(out.contains("\"upstream_error\""));
}

#[test]
fn maps_rich_events_to_status_messages() {
    let out =
        mapped_text("data: {\"type\":\"server_tool_started\",\"tool\":\"cabinet.search\"}\n\n");
    assert!(out.contains("\"message_type\":\"status_message\""));
    assert!(out.contains("Started cabinet.search"));
}

#[test]
fn maps_typed_runtime_card_without_losing_tool_metadata() {
    let out = mapped_text(
        "data: {\"type\":\"runtime_card\",\"card_kind\":\"tool_activity\",\"label\":\"Read src/main.rs\",\"source\":\"den_runtime\",\"tool\":{\"id\":\"call-1\",\"name\":\"fs_read_file\",\"arguments\":{\"path\":\"src/main.rs\"}},\"run_id\":\"run-1\",\"delivery\":{\"persisted\":true,\"visible_to_user\":true,\"sent_to_model\":false,\"derived_context\":false},\"redaction\":{\"state\":\"none\"}}\n\n",
    );
    assert!(out.contains("\"message_type\":\"runtime_card\""));
    assert!(out.contains("\"card_kind\":\"tool_activity\""));
    assert!(out.contains("\"call-1\""));
    assert!(out.contains("src/main.rs"));
}

#[test]
fn drops_done_control_event() {
    let out = mapped_text("data: {\"type\":\"done\",\"outcome\":\"ok\"}\n\n");
    assert!(out.is_empty());
}

#[test]
fn maps_ephemeral_assistant_delta_suffix_to_empty() {
    let out = mapped_text("data: {\"type\":\"assistant_delta\",\"text\":\"Thinking…\"}\n\n");
    assert!(out.is_empty());
}

#[tokio::test]
async fn bear_channel_proxy_assistant_text_then_done_does_not_emit_incomplete_terminal() {
    let out = collect_bear_channel_proxy_output(vec![
        "data: {\"type\":\"assistant_delta\",\"text\":\"Hello from Bear\"}\n\n",
        "data: {\"type\":\"done\",\"outcome\":\"ok\"}\n\n",
    ])
    .await;
    assert!(out.contains("Hello from Bear"), "output was {out}");
    assert!(
        !out.contains("stream_incomplete_terminal"),
        "output was {out}"
    );
    assert!(!out.contains("Reply incomplete"), "output was {out}");
}

#[tokio::test]
async fn bear_channel_proxy_status_only_then_done_emits_incomplete_terminal() {
    let out = collect_bear_channel_proxy_output(vec![
        "data: {\"type\":\"status_progress\",\"text\":\"Thinking…\"}\n\n",
        "data: {\"type\":\"done\",\"outcome\":\"ok\"}\n\n",
    ])
    .await;
    assert!(out.contains("Thinking…"), "output was {out}");
    assert!(
        out.contains("stream_incomplete_terminal") || out.contains("Reply incomplete"),
        "output was {out}"
    );
}

#[test]
fn chrome_only_stream_gets_incomplete_terminal_error() {
    let bytes = browser_incomplete_terminal_error(
        Uuid::parse_str("f42114ea-99bd-48a7-818a-78d4e3d914be").unwrap(),
    );
    let text = String::from_utf8(bytes.to_vec()).expect("utf8");
    assert!(text.contains("stream_incomplete_terminal") || text.contains("Reply incomplete"));
    assert!(text.contains("f42114ea-99bd-48a7-818a-78d4e3d914be"));
}

#[test]
fn empty_terminal_error_includes_support_ref() {
    let bytes = browser_empty_terminal_error(
        Uuid::parse_str("f42114ea-99bd-48a7-818a-78d4e3d914be").unwrap(),
    );
    let text = String::from_utf8(bytes.to_vec()).expect("utf8");
    assert!(text.contains("stream_empty_terminal") || text.contains("error_message"));
    assert!(text.contains("f42114ea-99bd-48a7-818a-78d4e3d914be"));
}
