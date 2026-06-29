//! Pure conversation-id classification + normalization helpers.
//!
//! These predicates decide how a client-supplied conversation selection maps onto runtime
//! history/archive targets. They have no storage or session dependencies, so they live at
//! the runtime root and are shared by the runtime conversation layer and the adapter edge
//! (adapter runtime re-exports them for existing call sites).

use den_core::DenError;

pub fn is_valid_pending_acp_conversation_id(conversation_id: &str) -> bool {
    conversation_id.starts_with("new-")
        && conversation_id.len() <= 42
        && normalize_acp_conversation_id(Some(conversation_id)).is_ok()
}

pub fn is_native_runtime_conversation_id(conversation_id: &str) -> bool {
    conversation_id.starts_with("den-conv-")
}

pub fn is_acp_history_target(conversation_id: &str) -> bool {
    conversation_id == "default"
        || conversation_id.starts_with("conv-")
        || is_native_runtime_conversation_id(conversation_id)
}

pub fn is_acp_archive_target(conversation_id: &str) -> bool {
    conversation_id.starts_with("conv-") || is_native_runtime_conversation_id(conversation_id)
}

pub fn normalized_durable_acp_conversation_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| is_acp_history_target(s))
        .map(str::to_string)
}

pub fn normalize_acp_conversation_id(raw: Option<&str>) -> Result<String, DenError> {
    let s = raw.unwrap_or("default").trim();
    if s.is_empty() {
        return Ok("default".to_string());
    }
    let ok = s == "default"
        || (s.starts_with("conv-") && s.len() >= 8)
        || (s.starts_with("new-") && s.len() >= 8)
        || s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(s.to_string())
    } else {
        Err(DenError::ValidationError(format!(
            "invalid conversation_id (expected 'default', a runtime conv- id, or a pending new- id): {s}"
        )))
    }
}
