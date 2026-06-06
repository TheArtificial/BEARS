use serde_json::{json, Value};

use crate::{
    core::tools::{
        memory_write::{MemoryWriteEntryArguments, validate_memory_write_entry_semantics},
        session::DenToolInvocationContext,
        support::assess_unlabeled_memory_misuse,
    },
    errors::CustomError,
};

use super::memory_write::DEN_MEMORY_WRITE_ENTRY;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolSemanticWarning {
    pub code: &'static str,
    pub category: &'static str,
    pub message: String,
    pub confirmation_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolPreflight {
    Proceed,
    Warning(ToolSemanticWarning),
}

pub(crate) fn tool_warning_payload(tool_name: &str, warning: ToolSemanticWarning) -> Value {
    json!({
        "status": "warning",
        "tool_name": tool_name,
        "warning": {
            "code": warning.code,
            "category": warning.category,
            "message": warning.message,
            "confirmation_token": warning.confirmation_token,
        }
    })
}

pub(crate) fn prevalidate_tool_arguments(
    tool_name: &str,
    arguments: &Value,
    context: &DenToolInvocationContext,
) -> Result<ToolPreflight, CustomError> {
    match tool_name {
        DEN_MEMORY_WRITE_ENTRY => {
            let args: MemoryWriteEntryArguments = serde_json::from_value(arguments.clone())?;
            validate_memory_write_entry_semantics(&args, context)?;
            assess_unlabeled_memory_misuse(&args, context)
        }
        _ => Ok(ToolPreflight::Proceed),
    }
}
