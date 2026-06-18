//! Den-owned model registry bootstrap.
//!
//! Den owns the user-facing model identities and capability metadata used by Bear
//! Admin. Bifrost remains the execution gateway, but its metadata sidecar is no
//! longer the UI source of truth.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::agent_assist::ModelOption;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct DenModelRegistryEntry {
    pub key: &'static str,
    pub provider: &'static str,
    pub provider_model_id: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub context_window: u32,
    pub max_output_tokens: Option<u32>,
    pub supports_tools: bool,
    pub supports_responses_api: bool,
    pub supports_vision: bool,
    /// Simple lifecycle: selectable entries appear in Bear Admin. Non-selectable
    /// entries may still resolve by key/alias for compatibility later.
    pub selectable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelGatewayCompatibilityReport {
    pub registry_selectable_count: usize,
    pub gateway_model_count: usize,
    pub missing_from_gateway: Vec<String>,
    pub unknown_gateway_models: Vec<String>,
    pub ok: bool,
}

impl DenModelRegistryEntry {
    pub fn to_model_option(self) -> ModelOption {
        ModelOption {
            handle: self.key.to_string(),
            label: model_label(self),
            context_window: Some(self.context_window),
            max_output_tokens: self.max_output_tokens,
        }
    }

    fn matches_handle(self, handle: &str) -> bool {
        let normalized = handle.trim();
        normalized == self.key
            || normalized == self.provider_model_id
            || self.aliases.iter().any(|alias| *alias == normalized)
    }
}

pub fn selectable_model_options() -> Vec<ModelOption> {
    let mut models = registry_entries()
        .into_iter()
        .filter(|entry| entry.selectable)
        .map(DenModelRegistryEntry::to_model_option)
        .collect::<Vec<_>>();
    models.sort_by(|a, b| a.label.cmp(&b.label));
    models
}

pub fn resolve_model_handle(handle: &str) -> Option<&'static str> {
    let trimmed = handle.trim();
    if trimmed.is_empty() {
        return None;
    }
    registry_entries()
        .into_iter()
        .find(|entry| entry.matches_handle(trimmed))
        .map(|entry| entry.key)
}

pub fn gateway_compatibility_report<I, S>(gateway_handles: I) -> ModelGatewayCompatibilityReport
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let registry = registry_entries();
    let selectable = registry
        .iter()
        .copied()
        .filter(|entry| entry.selectable)
        .collect::<Vec<_>>();
    let gateway = gateway_handles
        .into_iter()
        .map(|handle| handle.as_ref().trim().to_string())
        .filter(|handle| !handle.is_empty())
        .collect::<BTreeSet<_>>();

    let missing_from_gateway = selectable
        .iter()
        .filter(|entry| {
            !gateway.contains(entry.key) && !gateway.contains(entry.provider_model_id)
        })
        .map(|entry| entry.key.to_string())
        .collect::<Vec<_>>();

    let unknown_gateway_models = gateway
        .iter()
        .filter(|handle| registry.iter().copied().all(|entry| !entry.matches_handle(handle)))
        .cloned()
        .collect::<Vec<_>>();

    ModelGatewayCompatibilityReport {
        registry_selectable_count: selectable.len(),
        gateway_model_count: gateway.len(),
        ok: missing_from_gateway.is_empty(),
        missing_from_gateway,
        unknown_gateway_models,
    }
}

pub fn registry_entries() -> Vec<DenModelRegistryEntry> {
    vec![
        openai("openai/gpt-5.1", "gpt-5.1", "OpenAI GPT-5.1", &["gpt-5.1", "openai:gpt-5.1"], 400_000, Some(128_000), true, true),
        openai("openai/gpt-5", "gpt-5", "OpenAI GPT-5", &["gpt-5", "openai:gpt-5"], 400_000, Some(128_000), true, true),
        openai("openai/gpt-5-mini", "gpt-5-mini", "OpenAI GPT-5 mini", &["gpt-5-mini", "openai:gpt-5-mini"], 400_000, Some(128_000), true, true),
        openai("openai/gpt-5-nano", "gpt-5-nano", "OpenAI GPT-5 nano", &["gpt-5-nano", "openai:gpt-5-nano"], 400_000, Some(128_000), true, true),
        openai("openai/gpt-4.1", "gpt-4.1", "OpenAI GPT-4.1", &["gpt-4.1", "openai:gpt-4.1"], 1_047_576, Some(32_768), true, true),
        openai("openai/gpt-4.1-mini", "gpt-4.1-mini", "OpenAI GPT-4.1 mini", &["gpt-4.1-mini", "openai:gpt-4.1-mini"], 1_047_576, Some(32_768), true, true),
        openai("openai/gpt-4.1-nano", "gpt-4.1-nano", "OpenAI GPT-4.1 nano", &["gpt-4.1-nano", "openai:gpt-4.1-nano"], 1_047_576, Some(32_768), true, true),
        openai("openai/gpt-4o", "gpt-4o", "OpenAI GPT-4o", &["gpt-4o", "openai:gpt-4o"], 128_000, Some(16_384), true, true),
        openai("openai/gpt-4o-mini", "gpt-4o-mini", "OpenAI GPT-4o mini", &["gpt-4o-mini", "openai:gpt-4o-mini"], 128_000, Some(16_384), true, true),
        openai("openai/o4-mini", "o4-mini", "OpenAI o4-mini", &["o4-mini", "openai:o4-mini"], 200_000, Some(100_000), true, true),
        openai("openai/o3", "o3", "OpenAI o3", &["o3", "openai:o3"], 200_000, Some(100_000), true, true),
        openai("openai/o3-mini", "o3-mini", "OpenAI o3-mini", &["o3-mini", "openai:o3-mini"], 200_000, Some(100_000), true, false),
        openai("openai/o1", "o1", "OpenAI o1", &["o1", "openai:o1"], 200_000, Some(100_000), true, true),
        openai("openai/o1-mini", "o1-mini", "OpenAI o1-mini", &["o1-mini", "openai:o1-mini"], 128_000, Some(65_536), true, false),
    ]
}

fn openai(
    key: &'static str,
    provider_model_id: &'static str,
    display_name: &'static str,
    aliases: &'static [&'static str],
    context_window: u32,
    max_output_tokens: Option<u32>,
    supports_tools: bool,
    supports_vision: bool,
) -> DenModelRegistryEntry {
    DenModelRegistryEntry {
        key,
        provider: "openai",
        provider_model_id,
        display_name,
        aliases,
        context_window,
        max_output_tokens,
        supports_tools,
        supports_responses_api: true,
        supports_vision,
        selectable: true,
    }
}

fn model_label(entry: DenModelRegistryEntry) -> String {
    match entry.max_output_tokens {
        Some(out) => format!(
            "{} ({} ctx / {} out)",
            entry.display_name,
            format_tokens(entry.context_window),
            format_tokens(out)
        ),
        None => format!("{} ({} ctx)", entry.display_name, format_tokens(entry.context_window)),
    }
}

fn format_tokens(n: u32) -> String {
    if n >= 1_000_000 {
        let whole = n / 1_000_000;
        let frac = (n % 1_000_000) / 100_000;
        if frac == 0 {
            format!("{whole}M")
        } else {
            format!("{whole}.{frac}M")
        }
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectable_options_are_provider_qualified() {
        let options = selectable_model_options();
        assert!(options.iter().any(|m| m.handle == "openai/gpt-4.1"));
        assert!(options.iter().any(|m| m.handle == "openai/gpt-5"));
        assert!(options.iter().all(|m| m.handle.contains('/')));
    }

    #[test]
    fn resolves_aliases_to_canonical_keys() {
        assert_eq!(resolve_model_handle("gpt-4.1"), Some("openai/gpt-4.1"));
        assert_eq!(resolve_model_handle("openai:gpt-5"), Some("openai/gpt-5"));
        assert_eq!(resolve_model_handle("unknown-model"), None);
    }

    #[test]
    fn reports_gateway_compatibility() {
        let report = gateway_compatibility_report(["openai/gpt-4.1", "gpt-5"]);
        assert!(report.missing_from_gateway.iter().any(|h| h == "openai/gpt-4o"));
        assert!(report.unknown_gateway_models.is_empty());
    }

    #[test]
    fn labels_include_context() {
        let option = registry_entries()
            .into_iter()
            .find(|entry| entry.key == "openai/gpt-4.1")
            .expect("gpt-4.1 entry")
            .to_model_option();
        assert!(option.label.contains("1M ctx"));
    }
}
