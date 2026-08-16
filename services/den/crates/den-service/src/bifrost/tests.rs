use super::*;

#[test]
fn maps_live_models_response_to_metadata() {
    let payload: BifrostLiveModelsResponse = serde_json::from_str(
        r#"{
                "data": [{
                    "id": "openai/gpt-4.1",
                    "normalized_name": "GPT-4.1",
                    "context_length": 1047576,
                    "max_output_tokens": 32768,
                    "architecture": { "input_modalities": ["text", "image"] },
                    "supported_parameters": ["tools", "temperature", "reasoning_effort"],
                    "supported_methods": ["chat_completion", "responses"]
                }]
            }"#,
    )
    .expect("parse live models");
    let model = payload
        .data
        .into_iter()
        .next()
        .unwrap()
        .into_metadata()
        .unwrap();
    assert_eq!(model.handle, "openai/gpt-4.1");
    assert_eq!(model.provider, "openai");
    assert_eq!(model.model, "gpt-4.1");
    assert_eq!(model.display_name.as_deref(), Some("GPT-4.1"));
    assert_eq!(model.context_window, 1_047_576);
    assert_eq!(model.max_output_tokens, Some(32_768));
    assert_eq!(model.supports_tools, Some(true));
    assert_eq!(model.supports_responses_api, Some(true));
    assert_eq!(model.supports_vision, Some(true));
    assert_eq!(model.supports_reasoning_effort, Some(true));
}
