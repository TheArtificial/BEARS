    use super::*;

    #[test]
    fn shared_normal_records_are_indexable() {
        assert!(is_indexable("shared", "overview", "normal"));
        assert!(is_indexable("shared", "note", "normal"));
    }

    #[test]
    fn profile_local_only_indexes_select_kinds() {
        assert!(is_indexable("profile_local", "note", "normal"));
        assert!(is_indexable("profile_local", "decision", "normal"));
        assert!(is_indexable("profile_local", "summary", "normal"));
        assert!(!is_indexable("profile_local", "overview", "normal"));
    }

    #[test]
    fn non_normal_visibility_and_ephemeral_kinds_excluded() {
        assert!(!is_indexable("shared", "overview", "hidden"));
        assert!(!is_indexable("shared", "scratch", "normal"));
        assert!(!is_indexable("shared", "log", "normal"));
        assert!(!is_indexable("unknown_scope", "note", "normal"));
    }

    #[test]
    fn point_id_is_deterministic_and_varies_by_chunk() {
        let bear = Uuid::nil();
        let a = point_id(bear, "mem-1", 0, "bears-embed-v1");
        let a2 = point_id(bear, "mem-1", 0, "bears-embed-v1");
        let b = point_id(bear, "mem-1", 1, "bears-embed-v1");
        let c = point_id(bear, "mem-2", 0, "bears-embed-v1");
        assert_eq!(a, a2);
        assert_ne!(a, b);
        assert_ne!(a, c);
        // Valid UUID format.
        assert!(Uuid::parse_str(&a).is_ok());
    }

    #[test]
    fn payload_carries_required_fields() {
        let req = IndexRequest {
            bear_id: Uuid::nil(),
            memory_id: "mem-1".into(),
            logical_path: Some("core/work_surfaces/x/overview.md".into()),
            scope_type: "shared".into(),
            scope_profile: None,
            work_surface_ref: Some("x".into()),
            kind: "overview".into(),
            visibility: "normal".into(),
            content_text: "body".into(),
            entity_ids: vec!["ent-1".into(), "ent-2".into()],
        };
        let chunk = Chunk {
            index: 0,
            text: "body".into(),
            content_hash: "abc".into(),
        };
        let payload = build_payload(&req, &chunk, "bears-embed-v1");
        assert_eq!(payload["source_class"], SOURCE_CLASS_BEAR_MEMORY);
        assert_eq!(payload["embedding_standard"], "bears-embed-v1");
        assert_eq!(payload["memory_id"], "mem-1");
        assert_eq!(payload["chunk_index"], 0);
        assert_eq!(payload["content_hash"], "abc");
        assert_eq!(payload["work_surface_ref"], "x");
        assert_eq!(payload["kind"], "overview");
        assert_eq!(payload["text"], "body");
        assert_eq!(payload["entity_ids"], serde_json::json!(["ent-1", "ent-2"]));
    }
