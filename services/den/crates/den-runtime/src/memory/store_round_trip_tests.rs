#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use den_core::config::Config;
    use crate::memory::{
        store::{self, LogicalMemoryPath, MemoryStoreManager},
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
    async fn sqlite_entity_relation_round_trip() {
        let mut config = Config::test_stub();
        config.bear_sqlite_data_dir = format!("/tmp/bears-sqlite-relations-{}", Uuid::new_v4());
        let stores = MemoryStoreManager::new(&config);
        let bear_id = Uuid::new_v4();
        let store = stores.store_for_bear(bear_id).await.expect("store");

        let entity_id = match store::resolve(
            &store,
            "person",
            Some("Ryan"),
            &[store::Signal::new("email", "ryan@acme.com")],
            store::Assertion::Inferred,
        )
        .await
        .expect("resolve")
        {
            store::Resolution::Resolved(e) => e.entity_id,
            other => panic!("expected Resolved, got {other:?}"),
        };

        let src_id = "src-memory-1";
        let rel = store::append_relation(
            &store,
            src_id,
            &entity_id,
            "subject",
            &serde_json::json!({ "is_primary": true }),
            "pair",
            None,
            None,
        )
        .await
        .expect("append relation");
        assert_eq!(rel.relation, "den.memory.relation.subject");

        let links = store::list_relations_for_source(&store, src_id, 10)
            .await
            .expect("list relations");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].entity_id, entity_id);
    }
}
