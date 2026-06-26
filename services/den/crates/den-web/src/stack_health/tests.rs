    use super::count_usable_bifrost_models;

    #[test]
    fn bifrost_model_count_filters_routing_wildcards() {
        let value = serde_json::json!({
            "data": [
                { "id": "openai/*" },
                { "id": "*" },
                { "id": "openai/gpt-5.5" },
                { "id": "openai/gpt-4.1" }
            ]
        });

        assert_eq!(count_usable_bifrost_models(&value), (2, 2));
    }

    #[test]
    fn bifrost_model_count_treats_wildcard_only_as_no_usable_models() {
        let value = serde_json::json!({
            "data": [
                { "id": "openai/*" },
                { "id": "*" }
            ]
        });

        assert_eq!(count_usable_bifrost_models(&value), (0, 2));
    }
