use crate::api::acp::stream::mapping::runtime_stream_event_to_acp_seed_value;
use den_runtime::runtime_provider::RuntimeStreamEvent;

#[test]
fn untranslated_provider_event_passes_through_seed_value() {
    let value = serde_json::json!({
        "message_type": "tool_return_message",
        "tool_call_id": "call-1",
        "content": "ok"
    });
    let projected = runtime_stream_event_to_acp_seed_value(
        RuntimeStreamEvent::UntranslatedProviderEvent {
            value: value.clone(),
        },
    )
    .expect("projection should succeed");

    assert_eq!(projected, value);
}
