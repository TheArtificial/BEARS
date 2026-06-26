    use super::*;

    #[test]
    fn compact_client_tool_result_truncates_large_content_for_model() {
        let long = "x".repeat(40 * 1024);
        let compacted = compact_client_tool_result_params(
            "call_large",
            "ok",
            &json!({
                "content": long,
                "structured_content": { "nested": "y".repeat(40 * 1024) },
            }),
        );

        assert_eq!(compacted.payload["tool_call_id"], "call_large");
        assert_eq!(compacted.payload["result_compaction"]["truncated"], true);
        assert!(compacted.content.contains("tool result truncated for model context"));
        assert!(compacted.content.chars().count() < 25 * 1024);
    }

    #[test]
    fn compact_json_tool_result_truncates_large_den_hosted_result() {
        let compacted = compact_json_tool_result(json!({
            "results": [{ "body": "x".repeat(40 * 1024) }]
        }));

        assert_eq!(compacted.payload["result_compaction"]["truncated"], true);
        assert!(compacted.content.contains("tool result truncated for model context"));
        assert!(compacted.content.chars().count() < 25 * 1024);
    }
