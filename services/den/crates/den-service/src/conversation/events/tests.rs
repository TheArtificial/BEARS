    use super::*;

    #[test]
    fn canonical_persistence_enabled_for_den_conv_ids() {
        assert!(canonical_persistence_enabled_for_conversation("default"));
        assert!(canonical_persistence_enabled_for_conversation(
            "conv-abc123"
        ));
        assert!(canonical_persistence_enabled_for_conversation(
            "den-conv-abc123"
        ));
        assert!(!canonical_persistence_enabled_for_conversation(
            "provider-only-id"
        ));
    }
