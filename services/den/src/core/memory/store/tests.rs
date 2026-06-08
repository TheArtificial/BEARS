#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::config::Config;
    use crate::core::memory::{
        store::{LogicalMemoryPath, MemoryStoreManager},
        tools as sqlite_tools,
    };

    #[tokio::test]
    async fn sqlite_memory_round_trip() {
        let mut config = Config::test_stub();
        config.agent_runtime_mode = crate::config::AgentRuntimeMode::Native;
        config.bear_sqlite_data_dir = format!("/tmp/bears-sqlite-test-{}", Uuid::new_v4());
        let stores = MemoryStoreManager::new(&config);
        let bear_id = Uuid::new_v4();
        let written = sqlite_tools::sqlite_write_role_entry(
            &stores,
            &config,
            bear_id,
            "pair",
            "note",
            "Test",
            "Body",
            &[],
            None,
            Some("tester".to_string()),
        )
        .await
        .expect("write");
        let path = written
            .get("path")
            .and_then(|v| v.as_str())
            .expect("path");
        let store = stores.store_for_bear(bear_id).await.expect("store");
        let read = sqlite_tools::sqlite_memory_read(&store, path)
            .await
            .expect("read");
        assert!(read
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("Body"));
        let logical = LogicalMemoryPath::from_logical_path(path);
        assert_eq!(logical.scope_role.as_deref(), Some("pair"));
    }
}
