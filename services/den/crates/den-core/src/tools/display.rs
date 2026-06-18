//! Display metadata for model-facing tools.
//!
//! `ToolDisplayDescriptor` is the static presentation shape (labels, progress
//! verbs, target/sensitive argument keys, approval summary) consumed by ACP-facing
//! descriptors and the built-in Den tool descriptor table. It lives here, in the
//! descriptor authority crate, so both `den-tools` descriptors and the `den`-side
//! ACP tool surface share a single definition.

use serde_json::json;

#[derive(Debug, Clone, Copy)]
pub struct ToolDisplayDescriptor {
    pub label: &'static str,
    pub category: &'static str,
    pub progress_verb: &'static str,
    pub complete_verb: &'static str,
    pub target_arg_keys: &'static [&'static str],
    pub sensitive_arg_keys: &'static [&'static str],
    pub approval_summary: &'static str,
}

impl ToolDisplayDescriptor {
    pub fn to_json(self) -> serde_json::Value {
        json!({
            "label": self.label,
            "category": self.category,
            "progress_verb": self.progress_verb,
            "complete_verb": self.complete_verb,
            "target_arg_keys": self.target_arg_keys,
            "sensitive_arg_keys": self.sensitive_arg_keys,
            "approval_summary": self.approval_summary,
        })
    }
}
