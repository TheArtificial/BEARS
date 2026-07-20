use super::{ClientSessionMode, UPSERT_SESSION_SQL};

#[test]
fn client_session_mode_preserves_storage_strings_and_rejects_unknown_values() {
    assert_eq!(ClientSessionMode::Ask.as_str(), "ask");
    assert_eq!(ClientSessionMode::Plan.as_str(), "plan");
    assert_eq!(ClientSessionMode::Write.as_str(), "write");

    assert_eq!(
        ClientSessionMode::try_from_storage("ask").unwrap(),
        ClientSessionMode::Ask
    );
    assert_eq!(
        ClientSessionMode::try_from_storage("plan").unwrap(),
        ClientSessionMode::Plan
    );
    assert_eq!(
        ClientSessionMode::try_from_storage("write").unwrap(),
        ClientSessionMode::Write
    );
    assert!(ClientSessionMode::try_from_storage("admin").is_err());
}

#[test]
fn upsert_session_reopens_poisoned_closed_or_archived_rows() {
    assert!(
            UPSERT_SESSION_SQL.contains("closed_at = NULL"),
            "session.open/run.start upsert must clear closed_at so a previously closed client session can be reused"
        );
    assert!(
            UPSERT_SESSION_SQL.contains("archived_at = NULL"),
            "session.open/run.start upsert must clear archived_at so archived session rows cannot poison future turns"
        );
}

#[test]
fn trusted_workspace_context_prefers_adapter_roots_and_falls_back_to_cwd() {
    let row = super::ClientSessionRow {
        id: uuid::Uuid::nil(),
        user_id: 1,
        bear_id: uuid::Uuid::nil(),
        bear_slug: "bear".to_string(),
        client_session_id: "session-1".to_string(),
        runtime_session_id: "runtime-1".to_string(),
        conversation_id: "conv-1".to_string(),
        resolved_conversation_id: None,
        client: "zed".to_string(),
        cwd: Some("/workspace/cwd".to_string()),
        adapter_environment: Some(serde_json::json!({
            "cwd": "/workspace/cwd",
            "workspace_roots": ["/workspace/root-a", "/workspace/root-b"]
        })),
        current_mode: "ask".to_string(),
        conversation_title: None,
        conversation_title_updated_at: None,
        conversation_title_synced_at: None,
        closed_at: None,
        archived_at: None,
        created_at: time::OffsetDateTime::UNIX_EPOCH,
        updated_at: time::OffsetDateTime::UNIX_EPOCH,
    };

    let trusted = row.trusted_workspace_context();
    assert_eq!(trusted.cwd.as_deref(), Some("/workspace/cwd"));
    assert_eq!(
        trusted.roots,
        vec![
            "/workspace/root-a".to_string(),
            "/workspace/root-b".to_string()
        ]
    );
    assert_eq!(trusted.source, "trusted_session");

    let row = super::ClientSessionRow {
        adapter_environment: None,
        ..row
    };
    let trusted = row.trusted_workspace_context();
    assert_eq!(trusted.cwd.as_deref(), Some("/workspace/cwd"));
    assert_eq!(trusted.roots, vec!["/workspace/cwd".to_string()]);
    assert_eq!(trusted.source, "trusted_session");
}
