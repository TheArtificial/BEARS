    use super::*;

    #[test]
    fn from_config_none_when_recall_disabled() {
        let cfg = Config::test_stub();
        assert!(cfg.qdrant_url.is_none());
        assert!(QdrantRecall::from_config(&cfg).is_none());
    }

    #[test]
    fn from_config_some_with_derived_collection_name() {
        let mut cfg = Config::test_stub();
        cfg.qdrant_url = Some("http://bears-qdrant:6333/".to_string());
        cfg.embedding_standard = "bears-embed-v1".into();
        cfg.embedding_dimensions = 1536;

        let recall = QdrantRecall::from_config(&cfg).expect("recall client");
        assert_eq!(recall.base_url(), "http://bears-qdrant:6333");
        assert_eq!(recall.collection_name(), "den_recall_bears-embed-v1");
        assert_eq!(recall.dimensions, 1536);
    }

    #[test]
    fn collection_name_tracks_standard() {
        assert_eq!(collection_name("bears-embed-v2"), "den_recall_bears-embed-v2");
    }
