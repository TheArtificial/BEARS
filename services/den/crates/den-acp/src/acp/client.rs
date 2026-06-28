use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use uuid::Uuid;

use crate::{
    acp::AcpPromptRequest,
    core::{
        tools::{
        descriptor::builtin_den_tool_descriptors_for_pair_acp_surface,
        legacy_memory_tools::filter_client_tools_for_native_runtime,
    },
    },
};
use den_http::errors::CustomError;

pub(crate) fn tools_enabled_for_client(client: &str) -> bool {
    let normalized = normalize_acp_client(Some(client));
    matches!(
        normalized.as_str(),
        "zed" | "cursor" | "vscode" | "windsurf"
    )
}

pub(crate) fn normalize_acp_requested_mode(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ask" => Some("ask"),
        "plan" => Some("plan"),
        "write" => Some("write"),
        _ => None,
    }
}

pub(crate) fn requested_mode_from_prompt(
    body: &AcpPromptRequest,
) -> Result<Option<&'static str>, CustomError> {
    let Some(raw) = body
        .requested_mode
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    normalize_acp_requested_mode(raw).map(Some).ok_or_else(|| {
        CustomError::ValidationError("requested_mode must be one of ask, plan, write".to_string())
    })
}

pub(crate) fn normalize_acp_client(raw: Option<&str>) -> String {
    let value = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("acp_adapter");
    match value.to_ascii_lowercase().as_str() {
        "zed" => "zed".to_string(),
        "opencode" => "opencode".to_string(),
        _ => "acp_adapter".to_string(),
    }
}

pub(crate) fn new_acp_conversation_id(client: &str) -> String {
    let uuid = Uuid::new_v4();
    format!(
        "new-acp-{client}-{}",
        URL_SAFE_NO_PAD.encode(uuid.as_bytes())
    )
}

pub(crate) fn acp_pair_den_tool_descriptors() -> serde_json::Value {
    let descriptors = builtin_den_tool_descriptors_for_pair_acp_surface()
        .into_iter()
        .map(|descriptor| {
            serde_json::json!({
                "name": descriptor.provider_name,
                "description": format!(
                    "Den server tool ({}). {}",
                    descriptor.name, descriptor.description
                ),
                "parameters": descriptor.input_schema,
                "x-bears-domain": descriptor.domain,
                "x-bears-content-class": descriptor.content_class,
                "x-bears-display": descriptor.display,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!(descriptors)
}

pub(crate) fn merge_acp_pair_tool_descriptors(
    client_tools: serde_json::Value,
    native_runtime: bool,
) -> serde_json::Value {
    let client_tools = if native_runtime {
        filter_client_tools_for_native_runtime(Some(&client_tools)).unwrap_or(client_tools)
    } else {
        client_tools
    };
    let mut merged = client_tools.as_array().cloned().unwrap_or_default();
    if let Some(server_tools) = acp_pair_den_tool_descriptors().as_array() {
        merged.extend(server_tools.iter().cloned());
    }
    serde_json::json!(merged)
}
