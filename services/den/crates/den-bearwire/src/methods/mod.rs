use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
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

pub(crate) fn parse_params<T: DeserializeOwned>(
    params: &Value,
) -> Result<T, den_http::errors::CustomError> {
    serde_json::from_value(params.clone()).map_err(|err| {
        den_http::errors::CustomError::ValidationError(format!(
            "invalid BearWire params: {err}"
        ))
    })
}

pub(crate) fn deserialize_optional_i64_from_value<'de, D>(
    deserializer: D,
) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }))
}

pub(crate) fn deserialize_required_string<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(serde::de::Error::custom("expected non-empty string"));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn deserialize_optional_string<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

pub(crate) fn deserialize_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_required_string(deserializer)
}
