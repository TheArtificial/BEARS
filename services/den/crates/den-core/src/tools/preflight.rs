//! Tool-argument preflight: semantic warnings + the memory-write misuse gate.
//!
//! Pure (errors are `DenError`); moved out of `den` so the dispatcher seam can
//! follow. Re-exported at `core::tools::preflight::*` in the `den` crate.

use serde_json::{json, Value};

use crate::DenError;

use crate::tools::{
    constants::DEN_MEMORY_WRITE_ENTRY,
    context::DenToolInvocationContext,
    memory::MemoryWriteEntryArguments,
    support::{assess_unlabeled_memory_misuse, validate_memory_write_entry_semantics},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSemanticWarning {
    pub code: &'static str,
    pub category: &'static str,
    pub message: String,
    pub confirmation_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPreflight {
    Proceed,
    Warning(ToolSemanticWarning),
}

pub fn tool_warning_payload(tool_name: &str, warning: ToolSemanticWarning) -> Value {
    let ToolSemanticWarning {
        code,
        category,
        message,
        confirmation_token,
    } = warning;
    json!({
        "status": "warning",
        "tool_name": tool_name,
        "warning": {
            "code": code,
            "category": category,
            "message": message,
            "confirmation_token": confirmation_token,
        }
    })
}

pub fn prevalidate_tool_arguments(
    tool_name: &str,
    arguments: &Value,
    context: &DenToolInvocationContext,
) -> Result<ToolPreflight, DenError> {
    match tool_name {
        DEN_MEMORY_WRITE_ENTRY => {
            let args: MemoryWriteEntryArguments = serde_json::from_value(arguments.clone())?;
            validate_memory_write_entry_semantics(&args, context)?;
            assess_unlabeled_memory_misuse(&args, context)
        }
        _ => Ok(ToolPreflight::Proceed),
    }
}
