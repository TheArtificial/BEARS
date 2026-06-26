    use super::*;

    #[test]
    fn profile_parse_and_display_round_trip() {
        for profile in BearProfile::ALL {
            let parsed: BearProfile = profile.as_str().parse().expect("profile parses");
            assert_eq!(parsed, profile);
            assert_eq!(profile.to_string(), profile.as_str());
        }
        assert!("unknown".parse::<BearProfile>().is_err());
        assert!("talk".parse::<BearProfile>().is_err());
    }

    #[test]
    fn profile_runtime_family_matches_harness_design() {
        assert!(BearProfile::Chat.is_harness_backed());
        assert!(BearProfile::Work.is_harness_backed());
        assert!(!BearProfile::Pair.is_harness_backed());
        assert!(!BearProfile::Curate.is_harness_backed());
        assert!(!BearProfile::Watch.is_harness_backed());
        assert_eq!(BearProfile::Chat.runtime_family(), "native_harness_backed");
        assert_eq!(BearProfile::Work.runtime_family(), "native_harness_backed");
        assert_eq!(BearProfile::Pair.runtime_family(), "native_api_direct");
    }

    #[test]
    fn profile_tags_include_bear_profile_and_git_memory() {
        let bear_id = Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap();
        let tags = BearProfile::Work.tags_for_bear(den_core::BearId::from(bear_id));
        assert!(tags.contains(&format!("bear:{bear_id}")));
        assert!(tags.contains(&"profile:work".to_string()));
        assert!(tags.contains(&format!("bear:{bear_id}:profile:work")));
        assert!(tags.contains(&"git-memory-enabled".to_string()));
    }
