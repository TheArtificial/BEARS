//! Pure, shared validators for tool arguments.
//!
//! These return [`DenError`] (the web-free error); the `den` web boundary maps
//! that to `CustomError`. Re-exported at `crate::tools::core::tools::support::*` in the
//! `den` crate so existing callers keep resolving while executors migrate.

use crate::DenError;
use serde_json::Value;

/// Trim `value` and require its character count to be within `[min_chars, max_chars]`.
pub fn validate_bounded_text(
    field: &str,
    value: &str,
    min_chars: usize,
    max_chars: usize,
) -> Result<String, DenError> {
    let trimmed = value.trim();
    let char_count = trimmed.chars().count();
    if char_count < min_chars {
        return Err(DenError::ValidationError(format!("{field} must not be empty")));
    }
    if char_count > max_chars {
        return Err(DenError::ValidationError(format!(
            "{field} must be at most {max_chars} characters"
        )));
    }
    Ok(trimmed.to_string())
}

/// Require an optional JSON value, when present, to be an object.
pub fn validate_optional_object(field: &str, value: &Option<Value>) -> Result<(), DenError> {
    if let Some(value) = value {
        if !value.is_object() {
            return Err(DenError::ValidationError(format!("{field} must be an object")));
        }
    }
    Ok(())
}
