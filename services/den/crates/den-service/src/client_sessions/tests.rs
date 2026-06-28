    use super::UPSERT_SESSION_SQL;

    #[test]
    fn upsert_session_reopens_poisoned_closed_or_archived_rows() {
        assert!(
            UPSERT_SESSION_SQL.contains("closed_at = NULL"),
            "session.open/run.start upsert must clear closed_at so a previously closed ACP session can be reused"
        );
        assert!(
            UPSERT_SESSION_SQL.contains("archived_at = NULL"),
            "session.open/run.start upsert must clear archived_at so archived session rows cannot poison future turns"
        );
    }
