
use super::*;
use time::OffsetDateTime;

fn test_bear() -> Bear {
    Bear {
        id: Uuid::nil(),
        slug: "builder".to_string(),
        name: "Builder Bear".to_string(),
        description: String::new(),
        default_model: Some("openai/gpt-4o".to_string()),
        tools_enabled: None,
        runtime_plan: None,
        context_profile: None,
        provisioning_version: 1,
        system_prompt: String::new(),
        birthday: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
    }
}

#[test]
fn content_hash_is_deterministic() {
    assert_eq!(content_hash("abc"), content_hash("abc"));
    assert_ne!(content_hash("abc"), content_hash("abcd"));
}

#[test]
fn managed_space_block_key_matches_roles() {
    assert_eq!(
        managed_space_block_key(BearProfile::Chat),
        "space_instruction.chat"
    );
    assert_eq!(
        managed_space_block_key(BearProfile::Pair),
        "space_instruction.pair"
    );
    assert_eq!(
        managed_space_block_key(BearProfile::Curate),
        "space_instruction.curate"
    );
    assert_eq!(
        managed_space_block_key(BearProfile::Work),
        "space_instruction.work"
    );
    assert_eq!(
        managed_space_block_key(BearProfile::Watch),
        "space_instruction.watch"
    );
}

#[test]
fn seed_data_contains_expected_blocks_in_order() {
    let blocks = system_block_seed_data();
    let keys: Vec<&str> = blocks.iter().map(|b| b.key).collect();
    assert_eq!(
        keys,
        vec![
            "den_baseline",
            "space_instruction.chat",
            "space_instruction.pair",
            "space_instruction.curate",
            "space_instruction.work",
            "space_instruction.watch",
        ]
    );
}

#[test]
fn resolved_blocks_json_serializes() {
    let resolved = ResolvedManagedBlockSet {
        bear_id: test_bear().id,
        blocks: vec![ResolvedManagedBlock {
            key: "den_baseline".to_string(),
            kind: "prompt_text".to_string(),
            scope: "global".to_string(),
            source_mode: "inherit".to_string(),
            effective_content: "hello".to_string(),
            effective_content_hash: content_hash("hello"),
            system_version_id: Some(1),
            system_version_number: Some(1),
            forked_from_version_id: None,
            last_reviewed_version_id: None,
        }],
    };
    let json = resolved_blocks_json(&resolved).unwrap();
    assert_eq!(json.0["blocks"][0]["key"], "den_baseline");
}
