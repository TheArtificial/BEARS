use serde_json::{json, Value};

use den_service::DenState;

pub(crate) mod client;
pub(crate) mod conversation;
pub(crate) mod resource;
pub(crate) mod run;
pub(crate) mod session;

#[cfg(test)]
mod tests;

pub(crate) fn initialize_result(_state: &DenState) -> Value {
    json!({
        "protocol": "bearwire",
        "version": 1,
        "server": {
            "name": "den",
            "version": den_http::build_info::snapshot().version,
            "git_sha": den_http::build_info::snapshot().git_sha,
        },
        "bearwire": {
            "rpc": "/bearwire/v1/rpc",
            "events": "/bearwire/v1/sessions/{session_id}/events"
        },
        "legacy_acp_enabled": false,
        "legacy_acp_deprecated": true,
        "legacy_acp_removal_phase": "removed",
    })
}

pub(crate) fn param_string(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn required_param_string(
    params: &Value,
    key: &str,
) -> Result<String, den_http::errors::CustomError> {
    param_string(params, key)
        .ok_or_else(|| den_http::errors::CustomError::ValidationError(format!("{key} is required")))
}
