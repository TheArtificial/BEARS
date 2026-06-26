use super::{bifrost_virtual_key_for_inference_from_row, BearBifrostVirtualKey};

fn row(id: Option<&str>, value: Option<&str>, encrypted: Option<String>) -> BearBifrostVirtualKey {
    BearBifrostVirtualKey {
        bear_id: uuid::Uuid::nil(),
        virtual_key_id: id.map(str::to_string),
        virtual_key_name: Some("test key".to_string()),
        virtual_key_value: value.map(str::to_string),
        virtual_key_value_encrypted: encrypted,
    }
}

#[test]
fn bifrost_inference_header_uses_virtual_key_secret_value_not_id() {
    let selected = bifrost_virtual_key_for_inference_from_row(
        row(Some("vk_123"), Some("sk-bf-secret"), None),
        "unused-secret-key",
    )
    .expect("select inference key");

    assert_eq!(selected.as_deref(), Some("sk-bf-secret"));
}

#[test]
fn bifrost_inference_header_falls_back_to_legacy_plaintext_value_without_id() {
    let selected = bifrost_virtual_key_for_inference_from_row(
        row(None, Some("sk-bf-legacy"), None),
        "unused-secret-key",
    )
    .expect("select inference key");

    assert_eq!(selected.as_deref(), Some("sk-bf-legacy"));
}

#[test]
fn bifrost_inference_header_ignores_id_when_selecting_secret_value() {
    let selected = bifrost_virtual_key_for_inference_from_row(
        row(Some("vk_123"), Some("sk-bf-legacy"), None),
        "unused-secret-key",
    )
    .expect("select inference key");

    assert_eq!(selected.as_deref(), Some("sk-bf-legacy"));
}
