//! Den-owned model registry bootstrap.
//!
//! This is the first implementation of the Den model registry plan: Den owns the
//! user-facing model identities and capability metadata used by Bear Admin. Bifrost
//! remains the execution gateway, but its metadata sidecar is no longer the UI source
//! of truth.

use crate::agent_assist::ModelOption;

#[derive(Debug, Clone, Copy)]
pub struct DenModelRegistryEntry {
    pub key: &'static str,
    pub provider: &'static str,
    pub provider_model_id: &'static str,
    pub display_name: &'static str,
    pub context_window: u32,
    pub max_output_tokens: Option<u32>,
    pub supports_tools: bool,
    pub supports_responses_api: bool,
    pub supports_vision: bool,
    pub enabled: bool,
    pub selectable: bool,
    pub deprecated: bool,
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
}

pub fn selectable_model_options() -> Vec<ModelOption> {
    let mut models = registry_entries()
        .into_iter()
        .filter(|entry| entry.enabled && entry.selectable && !entry.deprecated)
        .map(DenModelRegistryEntry::to_model_option)
        .collect::<Vec<_>>();
    models.sort_by(|a, b| a.label.cmp(&b.label));
    models
}

pub fn registry_entries() -> Vec<DenModelRegistryEntry> {
    vec![
        openai("openai/gpt-5.1", "gpt-5.1", "OpenAI GPT-5.1", 400_000, Some(128_000), true, true),
        openai("openai/gpt-5", "gpt-5", "OpenAI GPT-5", 400_000, Some(128_000), true, true),
        openai("openai/gpt-5-mini", "gpt-5-mini", "OpenAI GPT-5 mini", 400_000, Some(128_000), true, true),
        openai("openai/gpt-5-nano", "gpt-5-nano", "OpenAI GPT-5 nano", 400_000, Some(128_000), true, true),
        openai("openai/gpt-4.1", "gpt-4.1", "OpenAI GPT-4.1", 1_047_576, Some(32_768), true, true),
        openai("openai/gpt-4.1-mini", "gpt-4.1-mini", "OpenAI GPT-4.1 mini", 1_047_576, Some(32_768), true, true),
        openai("openai/gpt-4.1-nano", "gpt-4.1-nano", "OpenAI GPT-4.1 nano", 1_047_576, Some(32_768), true, true),
        openai("openai/gpt-4o", "gpt-4o", "OpenAI GPT-4o", 128_000, Some(16_384), true, true),
        openai("openai/gpt-4o-mini", "gpt-4o-mini", "OpenAI GPT-4o mini", 128_000, Some(16_384), true, true),
        openai("openai/o4-mini", "o4-mini", "OpenAI o4-mini", 200_000, Some(100_000), true, true),
        openai("openai/o3", "o3", "OpenAI o3", 200_000, Some(100_000), true, true),
        openai("openai/o3-mini", "o3-mini", "OpenAI o3-mini", 200_000, Some(100_000), true, false),
        openai("openai/o1", "o1", "OpenAI o1", 200_000, Some(100_000), true, true),
        openai("openai/o1-mini", "o1-mini", "OpenAI o1-mini", 128_000, Some(65_536), true, false),
    ]
}

fn openai(
    key: &'static str,
    provider_model_id: &'static str,
    display_name: &'static str,
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
        context_window,
        max_output_tokens,
        supports_tools,
        supports_responses_api: true,
        supports_vision,
        enabled: true,
        selectable: true,
        deprecated: false,
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
    fn labels_include_context() {
        let option = registry_entries()
            .into_iter()
            .find(|entry| entry.key == "openai/gpt-4.1")
            .expect("gpt-4.1 entry")
            .to_model_option();
        assert!(option.label.contains("1M ctx"));
    }
}
