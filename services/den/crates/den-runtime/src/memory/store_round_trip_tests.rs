#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use den_core::config::Config;
    use crate::memory::{
        store::{
            append_memory_link, list_memory_links_for_source, LogicalMemoryPath, MemoryStoreManager,
        },
        tools as sqlite_tools,
    };

    #[tokio::test]
    async fn sqlite_memory_round_trip() {
        let mut config = Config::test_stub();
        config.bear_sqlite_data_dir = format!("/tmp/bears-sqlite-test-{}", Uuid::new_v4());
        let stores = MemoryStoreManager::new(&config);
        let bear_id = Uuid::new_v4();
        let written = sqlite_tools::sqlite_write_profile_entry(
            &stores,
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
        assert_eq!(logical.scope_profile.as_deref(), Some("pair"));
    }

    #[tokio::test]
    async fn sqlite_memory_links_round_trip() {
        let mut config = Config::test_stub();
        config.bear_sqlite_data_dir = format!("/tmp/bears-sqlite-links-{}", Uuid::new_v4());
        let stores = MemoryStoreManager::new(&config);
        let bear_id = Uuid::new_v4();
        let store = stores.store_for_bear(bear_id).await.expect("store");
        let src_id = "src-memory-1";
        let link_id = append_memory_link(
            &store,
            src_id,
            "memory_record",
            "dst-memory-2",
            "promotion",
        )
        .await
        .expect("append link");
        assert!(!link_id.is_empty());
        let links = list_memory_links_for_source(&store, src_id, 10)
            .await
            .expect("list links");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].dst_ref, "dst-memory-2");
        assert_eq!(links[0].link_type, "promotion");
    }
}
