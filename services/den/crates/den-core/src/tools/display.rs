//! Display metadata for model-facing tools.
//!
//! `ToolDisplayDescriptor` is the static presentation shape (labels, progress
//! verbs, target/sensitive argument keys, approval summary) consumed by adapter-facing
//! descriptors and the built-in Den tool descriptor table. It lives here, in the
//! descriptor authority crate, so both `den-tools` descriptors and the `den`-side
//! armature tool surface share a single definition.

use serde_json::json;
use std::path::{Component, Path};

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

pub fn is_display_path_key(key: &str) -> bool {
    matches!(
        key,
        "path" | "repo_path" | "source_path" | "destination_path" | "root" | "base_path" | "cwd" | "target_path"
    )
}

pub fn display_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    let candidate = Path::new(trimmed);
    let is_absolute = candidate.is_absolute()
        || trimmed.starts_with("\\\\")
        || (trimmed.len() >= 3
            && trimmed.as_bytes()[0].is_ascii_alphabetic()
            && trimmed.as_bytes()[1] == b':'
            && matches!(trimmed.as_bytes()[2], b'/' | b'\\'));
    if !is_absolute {
        return trimmed.to_string();
    }
    let parts = candidate
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return trimmed.to_string();
    }
    let keep = 3.min(parts.len());
    format!("…/{}", parts[parts.len() - keep..].join("/"))
}
