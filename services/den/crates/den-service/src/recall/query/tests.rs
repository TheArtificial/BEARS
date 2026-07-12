    use super::*;

    fn passage(memory_id: &str, path: &str, score: f32) -> RecalledPassage {
        RecalledPassage {
            memory_id: memory_id.into(),
            logical_path: Some(path.into()),
            kind: Some("note".into()),
            score,
            salience: "normal".into(),
            text: "the quick brown fox jumps over the lazy dog".into(),
        }
    }

    #[test]
    fn render_drops_paths_already_in_anchors() {
        let projection = RecallProjection {
            passages: vec![
                passage("m1", "core/a.md", 0.91),
                passage("m2", "core/b.md", 0.80),
            ],
            diagnostic: Value::Null,
        };
        let anchors = "# Projected memory\n- core/a.md: ...";
        let block = render_recall_block(&projection, anchors).expect("block");
        assert!(block.contains("core/b.md"));
        assert!(!block.contains("core/a.md"));
    }

    #[test]
    fn render_none_when_all_deduped() {
        let projection = RecallProjection {
            passages: vec![passage("m1", "core/a.md", 0.91)],
            diagnostic: Value::Null,
        };
        assert!(render_recall_block(&projection, "core/a.md").is_none());
    }

    #[test]
    fn truncate_collapses_whitespace_and_caps_length() {
        let out = truncate_chars("a\n\n  b   c", 100);
        assert_eq!(out, "a b c");
        let long = "x".repeat(600);
        let capped = truncate_chars(&long, SNIPPET_CHARS);
        assert_eq!(capped.chars().count(), SNIPPET_CHARS + 1); // + ellipsis
    }

    #[test]
    fn role_scope_filter_requires_shared_or_own_role() {
        let bear = Uuid::nil();
        let filter = role_scope_filter(bear, "bears-embed-v1", "work");
        let must = filter["must"].as_array().expect("must array");
        // Three mandatory scope conditions + one nested should clause.
        assert_eq!(must.len(), 4, "{filter}");
        assert_eq!(must[0]["key"], "bear_id");
        assert_eq!(must[1]["match"]["value"], SOURCE_CLASS_BEAR_MEMORY);
        assert_eq!(must[2]["match"]["value"], "bears-embed-v1");
        let should = must[3]["should"].as_array().expect("nested should");
        assert_eq!(should[0]["key"], "scope_type");
        assert_eq!(should[0]["match"]["value"], "shared");
        assert_eq!(should[1]["key"], "scope_profile");
        assert_eq!(should[1]["match"]["value"], "work");
    }

    #[test]
    fn entity_scope_filter_requires_entity_membership() {
        let bear = Uuid::nil();
        let filter = entity_scope_filter(bear, "bears-embed-v1", &["e1".into(), "e2".into()]);
        let must = filter["must"].as_array().expect("must array");
        // Three mandatory scope conditions + one entity-membership clause.
        assert_eq!(must.len(), 4, "{filter}");
        assert_eq!(must[0]["key"], "bear_id");
        assert_eq!(must[3]["key"], "entity_ids");
        let any = must[3]["match"]["any"].as_array().expect("any array");
        assert_eq!(any.len(), 2);
        assert_eq!(any[0], "e1");
        assert_eq!(any[1], "e2");
    }

    #[test]
    fn merge_unions_vector_then_keyword_and_dedupes() {
        let vector = RecallProjection {
            passages: vec![passage("m1", "core/a.md", 0.91)],
            diagnostic: json!({ "status": "ok" }),
        };
        // Keyword leg re-surfaces m1 (must be deduped) and adds a unique m2.
        let keyword = json!({
            "hits": [
                { "memory_id": "m1", "path": "core/a.md", "kind": "note", "snippet": "dup" },
                { "memory_id": "m2", "path": "work/b.md", "kind": "note", "snippet": "exact match" },
            ]
        });
        let value = merge_search_results(&vector, &keyword, &[], "fox", 10);

        assert_eq!(value["storage"], "hybrid");
        assert_eq!(value["strategy"], "vector+keyword");
        let hits = value["hits"].as_array().expect("hits array");
        assert_eq!(hits.len(), 2, "m1 deduped, m2 appended: {hits:?}");
        // Vector hit ranks first and carries its score + source.
        assert_eq!(hits[0]["memory_id"], "m1");
        assert_eq!(hits[0]["source"], "vector");
        assert!((hits[0]["score"].as_f64().unwrap() - 0.91).abs() < 1e-6);
        assert!(hits[0]["snippet"].as_str().unwrap().contains("quick brown fox"));
        // Keyword-only hit follows, unranked.
        assert_eq!(hits[1]["memory_id"], "m2");
        assert_eq!(hits[1]["source"], "keyword");
        assert!(hits[1]["score"].is_null());
    }

    #[test]
    fn merge_strategy_reflects_contributing_legs() {
        let only_vector = merge_search_results(
            &RecallProjection { passages: vec![passage("m1", "core/a.md", 0.9)], diagnostic: Value::Null },
            &json!({ "hits": [] }),
            &[],
            "q",
            10,
        );
        assert_eq!(only_vector["strategy"], "vector");

        let only_keyword = merge_search_results(
            &RecallProjection { passages: vec![], diagnostic: Value::Null },
            &json!({ "hits": [{ "memory_id": "m9", "path": "p", "kind": "note", "snippet": "s" }] }),
            &[],
            "q",
            10,
        );
        assert_eq!(only_keyword["strategy"], "keyword");
    }

    #[test]
    fn merge_appends_graph_leg_after_direct_hits_and_dedupes() {
        let vector = RecallProjection {
            passages: vec![passage("m1", "core/a.md", 0.9)],
            diagnostic: Value::Null,
        };
        let keyword = json!({ "hits": [{ "memory_id": "m2", "path": "work/b.md", "kind": "note", "snippet": "kw" }] });
        // m1 is re-surfaced by the graph leg (must dedupe); m3 is a genuine 2-hop reach.
        let graph = vec![
            json!({ "memory_id": "m1", "path": "core/a.md", "kind": "note", "score": Value::Null, "snippet": "dup", "source": "graph", "hop": 1 }),
            json!({ "memory_id": "m3", "path": "core/c.md", "kind": "note", "score": Value::Null, "snippet": "reached via shared entity", "source": "graph", "hop": 2 }),
        ];
        let value = merge_search_results(&vector, &keyword, &graph, "q", 10);

        assert_eq!(value["strategy"], "vector+keyword+graph");
        let hits = value["hits"].as_array().expect("hits array");
        assert_eq!(hits.len(), 3, "m1 deduped against the vector hit: {hits:?}");
        assert_eq!(hits[0]["memory_id"], "m1");
        assert_eq!(hits[1]["memory_id"], "m2");
        // Graph-reached record ranks last and carries its provenance + hop distance.
        assert_eq!(hits[2]["memory_id"], "m3");
        assert_eq!(hits[2]["source"], "graph");
        assert_eq!(hits[2]["hop"], 2);
        assert!(hits[2]["score"].is_null());
    }

    #[test]
    fn merge_caps_to_limit_prioritizing_vector() {
        let vector = RecallProjection {
            passages: vec![passage("m1", "core/a.md", 0.9), passage("m2", "core/b.md", 0.8)],
            diagnostic: Value::Null,
        };
        let keyword = json!({ "hits": [{ "memory_id": "m3", "path": "p", "kind": "note", "snippet": "s" }] });
        let value = merge_search_results(&vector, &keyword, &[], "q", 2);
        let hits = value["hits"].as_array().expect("hits");
        assert_eq!(hits.len(), 2, "capped to limit");
        assert_eq!(hits[0]["memory_id"], "m1");
        assert_eq!(hits[1]["memory_id"], "m2", "vector hits retained over keyword when capping");
    }
