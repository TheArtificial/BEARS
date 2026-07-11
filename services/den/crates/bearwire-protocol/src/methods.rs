use serde::{Deserialize, Deserializer};
use serde_json::Value;

pub fn deserialize_optional_i64_from_value<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
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

pub fn deserialize_required_string<'de, D>(deserializer: D) -> Result<String, D::Error>
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

pub fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

pub fn deserialize_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_required_string(deserializer)
}

fn default_history_limit() -> i64 {
    50
}



#[derive(Debug, Deserialize)]
pub struct EventPageQuery {
    #[serde(deserialize_with = "deserialize_required_string")]
    pub bear_slug: String,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_optional_i64_from_value")]
    pub after: Option<i64>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_optional_i64_from_value")]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ConversationHistoryRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    pub conversation_id: String,
    #[serde(
        default,
        alias = "before_sequence_no",
        deserialize_with = "deserialize_optional_i64_from_value"
    )]
    pub before: Option<i64>,
    #[serde(default = "default_history_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub struct ResourceUpdateRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    pub session_id: String,
    /// Intentionally raw: adapter-owned resource envelopes are extensible and forwarded verbatim.
    #[serde(default)]
    pub resource: Option<Value>,
    /// Legacy alias for `resource`; intentionally raw for the same reason.
    #[serde(default)]
    pub payload: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct RunStartRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    pub session_id: String,
    #[serde(deserialize_with = "deserialize_required_string")]
    pub prompt: String,
    /// Intentionally raw: prompt context is a typed outer field carrying extensible structured payloads.
    #[serde(default)]
    pub prompt_context: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub client: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub conversation_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub cwd: Option<String>,
    #[serde(
        default,
        alias = "mode",
        deserialize_with = "deserialize_optional_string"
    )]
    pub requested_mode: Option<String>,
    /// Intentionally raw: adapter session/capability context is an extensible envelope.
    #[serde(default)]
    pub client_context: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct RunCancelRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SessionOpenRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    pub session_id: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub client: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub conversation_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub runtime_session_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub cwd: Option<String>,
    #[serde(
        default,
        alias = "requested_mode",
        deserialize_with = "deserialize_optional_string"
    )]
    pub mode: Option<String>,
    /// Intentionally raw: adapter session snapshots are open-ended capability/context envelopes.
    #[serde(default)]
    pub client_context: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct SessionIdRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SessionStateRequest {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub bear_slug: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub include_closed: Option<bool>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_optional_i64_from_value")]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SessionModelSetRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    pub session_id: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub selection_mode: Option<String>,
    #[serde(
        default,
        alias = "requested_model",
        deserialize_with = "deserialize_optional_string"
    )]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecisionInput {
    Approved,
    Approve,
    Granted,
    Allow,
    AllowOnce,
    AllowSiteAccount,
    AllowHost,
    Denied,
    Deny,
    Rejected,
    Reject,
    RejectOnce,
    RejectAlways,
    Timeout,
    TimedOut,
}

impl PermissionDecisionInput {
    pub fn normalized(self) -> &'static str {
        match self {
            Self::Approved
            | Self::Approve
            | Self::Granted
            | Self::Allow
            | Self::AllowOnce
            | Self::AllowSiteAccount
            | Self::AllowHost => "granted",
            Self::Denied
            | Self::Deny
            | Self::Rejected
            | Self::Reject
            | Self::RejectOnce
            | Self::RejectAlways => "denied",
            Self::Timeout | Self::TimedOut => "expired",
        }
    }

    pub fn raw(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Approve => "approve",
            Self::Granted => "granted",
            Self::Allow => "allow",
            Self::AllowOnce => "allow_once",
            Self::AllowSiteAccount => "allow_site_account",
            Self::AllowHost => "allow_host",
            Self::Denied => "denied",
            Self::Deny => "deny",
            Self::Rejected => "rejected",
            Self::Reject => "reject",
            Self::RejectOnce => "reject_once",
            Self::RejectAlways => "reject_always",
            Self::Timeout => "timeout",
            Self::TimedOut => "timed_out",
        }
    }
}

fn default_permission_decision() -> PermissionDecisionInput {
    PermissionDecisionInput::Denied
}

#[derive(Debug, Deserialize)]
pub struct ClientPermissionResultRequest {
    #[serde(deserialize_with = "deserialize_required_string")]
    pub run_id: String,
    #[serde(deserialize_with = "deserialize_required_string")]
    pub session_id: String,
    #[serde(deserialize_with = "deserialize_required_string")]
    pub permission_id: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub obligation_id: Option<String>,
    #[serde(default = "default_permission_decision")]
    pub decision: PermissionDecisionInput,
    /// Intentionally raw: reason may be string or structured adapter metadata.
    #[serde(default)]
    pub reason: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_strings_are_trimmed_and_empty_strings_rejected() {
        let request: RunCancelRequest =
            serde_json::from_value(serde_json::json!({ "session_id": " s-1 " })).unwrap();
        assert_eq!(request.session_id, "s-1");

        let error = serde_json::from_value::<RunCancelRequest>(serde_json::json!({
            "session_id": "   "
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains("expected non-empty string"));
    }

    #[test]
    fn method_aliases_and_numeric_strings_decode() {
        let history: ConversationHistoryRequest = serde_json::from_value(serde_json::json!({
            "conversation_id": "conv-1",
            "before_sequence_no": "42"
        }))
        .unwrap();
        assert_eq!(history.before, Some(42));
        assert_eq!(history.limit, 50);

        let run: RunStartRequest = serde_json::from_value(serde_json::json!({
            "session_id": "s-1",
            "prompt": "hi",
            "mode": " ask "
        }))
        .unwrap();
        assert_eq!(run.requested_mode.as_deref(), Some("ask"));
    }
}
