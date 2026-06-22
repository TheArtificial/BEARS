//! Den-owned model registry bootstrap.
//!
//! Den owns the user-facing model identities and capability metadata used by Bear
//! Admin. Bifrost remains the execution gateway, but its metadata sidecar is no
//! longer the UI source of truth.

use std::collections::{BTreeSet, HashMap};
use std::sync::{OnceLock, RwLock};

use serde::Serialize;

use crate::ModelOption;

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
    pub metadata_model_count: usize,
    pub gateway_model_count: usize,
    pub available_with_metadata: Vec<String>,
    pub available_without_metadata: Vec<String>,
    pub metadata_not_available: Vec<String>,
    pub ok: bool,
}

impl DenModelRegistryEntry {
    pub fn to_model_option(self) -> ModelOption {
        self.to_model_option_for_handle(self.key)
    }

    pub fn to_model_option_for_handle(self, handle: &str) -> ModelOption {
        ModelOption {
            handle: handle.to_string(),
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
    entry_for_handle(handle).map(|entry| entry.key)
}

pub fn provider_model_id_for_handle(handle: &str) -> Option<&'static str> {
    entry_for_handle(handle).map(|entry| entry.provider_model_id)
}

/// Gateway-advertised Responses-API support, keyed by canonical handle.
///
/// Populated from the live Bifrost `/v1/models` catalog (`supported_methods`)
/// rather than the static registry, so routing tracks whatever the gateway
/// currently serves. Empty until the catalog has been observed at least once.
fn catalog_responses_api_support() -> &'static RwLock<HashMap<String, bool>> {
    static CACHE: OnceLock<RwLock<HashMap<String, bool>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Canonical cache key for a handle: the registry `key` when known, otherwise
/// the `provider/model`-normalized form (mirrors `DenModelHandle::normalize`).
fn catalog_cache_key(handle: &str) -> String {
    let trimmed = handle.trim();
    if let Some(entry) = entry_for_handle(trimmed) {
        return entry.key.to_string();
    }
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains('/') {
        trimmed.to_string()
    } else {
        format!("openai/{trimmed}")
    }
}

/// Record what the Bifrost catalog advertised for a model's Responses-API
/// support. `None` (catalog stated nothing) leaves any prior record untouched.
pub fn record_catalog_responses_api_support(handle: &str, supports: Option<bool>) {
    let Some(supports) = supports else {
        return;
    };
    let key = catalog_cache_key(handle);
    if key.is_empty() {
        return;
    }
    if let Ok(mut cache) = catalog_responses_api_support().write() {
        cache.insert(key, supports);
    }
}

/// Gateway-advertised Responses-API support for a handle, if the catalog has
/// been observed for it; `None` before the catalog is loaded or for unknown
/// handles, so callers can fall back to a static default.
pub fn catalog_supports_responses_api(handle: &str) -> Option<bool> {
    let key = catalog_cache_key(handle);
    if key.is_empty() {
        return None;
    }
    catalog_responses_api_support()
        .read()
        .ok()
        .and_then(|cache| cache.get(&key).copied())
}

pub fn is_routing_wildcard_model_handle(handle: &str) -> bool {
    let trimmed = handle.trim();
    trimmed == "*" || trimmed.ends_with("/*")
}

pub fn entry_for_handle(handle: &str) -> Option<DenModelRegistryEntry> {
    let trimmed = handle.trim();
    if trimmed.is_empty() || is_routing_wildcard_model_handle(trimmed) {
        return None;
    }
    registry_entries()
        .into_iter()
        .find(|entry| entry.matches_handle(trimmed))
}

pub fn model_option_for_available_handle(
    handle: &str,
    fallback_label: Option<&str>,
    fallback_context_window: Option<u32>,
    fallback_max_output_tokens: Option<u32>,
) -> ModelOption {
    if let Some(entry) = entry_for_handle(handle) {
        return entry.to_model_option_for_handle(handle.trim());
    }

    let trimmed = handle.trim();
    debug_assert!(!is_routing_wildcard_model_handle(trimmed));
    let label_base = fallback_label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(trimmed);
    let label = match (fallback_context_window, fallback_max_output_tokens) {
        (Some(ctx), Some(out)) => format!(
            "{} ({} ctx / {} out) — metadata unknown",
            label_base,
            format_tokens(ctx),
            format_tokens(out)
        ),
        (Some(ctx), None) => format!(
            "{} ({} ctx) — metadata unknown",
            label_base,
            format_tokens(ctx)
        ),
        _ => format!("{label_base} — metadata unknown"),
    };
    ModelOption {
        handle: trimmed.to_string(),
        label,
        context_window: fallback_context_window,
        max_output_tokens: fallback_max_output_tokens,
    }
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
        .filter(|handle| !handle.is_empty() && !is_routing_wildcard_model_handle(handle))
        .collect::<BTreeSet<_>>();

    let available_with_metadata = gateway
        .iter()
        .filter(|handle| {
            registry
                .iter()
                .copied()
                .any(|entry| entry.matches_handle(handle))
        })
        .cloned()
        .collect::<Vec<_>>();

    let available_without_metadata = gateway
        .iter()
        .filter(|handle| {
            registry
                .iter()
                .copied()
                .all(|entry| !entry.matches_handle(handle))
        })
        .cloned()
        .collect::<Vec<_>>();

    let metadata_not_available = selectable
        .iter()
        .filter(|entry| {
            !gateway.contains(entry.key)
                && !gateway.contains(entry.provider_model_id)
                && entry.aliases.iter().all(|alias| !gateway.contains(*alias))
        })
        .map(|entry| entry.key.to_string())
        .collect::<Vec<_>>();

    ModelGatewayCompatibilityReport {
        metadata_model_count: selectable.len(),
        gateway_model_count: gateway.len(),
        ok: available_without_metadata.is_empty(),
        available_with_metadata,
        available_without_metadata,
        metadata_not_available,
    }
}

pub fn registry_entries() -> Vec<DenModelRegistryEntry> {
    vec![
        openai(
            "openai/gpt-5.5",
            "gpt-5.5",
            "OpenAI GPT-5.5",
            &["gpt-5.5", "openai:gpt-5.5"],
            400_000,
            Some(128_000),
            true,
            true,
        ),
        openai(
            "openai/gpt-5.1",
            "gpt-5.1",
            "OpenAI GPT-5.1",
            &["gpt-5.1", "openai:gpt-5.1"],
            400_000,
            Some(128_000),
            true,
            true,
        ),
        openai(
            "openai/gpt-5",
            "gpt-5",
            "OpenAI GPT-5",
            &["gpt-5", "openai:gpt-5"],
            400_000,
            Some(128_000),
            true,
            true,
        ),
        openai(
            "openai/gpt-5-mini",
            "gpt-5-mini",
            "OpenAI GPT-5 mini",
            &["gpt-5-mini", "openai:gpt-5-mini"],
            400_000,
            Some(128_000),
            true,
            true,
        ),
        openai(
            "openai/gpt-5-nano",
            "gpt-5-nano",
            "OpenAI GPT-5 nano",
            &["gpt-5-nano", "openai:gpt-5-nano"],
            400_000,
            Some(128_000),
            true,
            true,
        ),
        openai(
            "openai/gpt-4.1",
            "gpt-4.1",
            "OpenAI GPT-4.1",
            &["gpt-4.1", "openai:gpt-4.1"],
            1_047_576,
            Some(32_768),
            true,
            true,
        ),
        openai(
            "openai/gpt-4.1-mini",
            "gpt-4.1-mini",
            "OpenAI GPT-4.1 mini",
            &["gpt-4.1-mini", "openai:gpt-4.1-mini"],
            1_047_576,
            Some(32_768),
            true,
            true,
        ),
        openai(
            "openai/gpt-4.1-nano",
            "gpt-4.1-nano",
            "OpenAI GPT-4.1 nano",
            &["gpt-4.1-nano", "openai:gpt-4.1-nano"],
            1_047_576,
            Some(32_768),
            true,
            true,
        ),
        openai(
            "openai/gpt-4o",
            "gpt-4o",
            "OpenAI GPT-4o",
            &["gpt-4o", "openai:gpt-4o"],
            128_000,
            Some(16_384),
            true,
            true,
        ),
        openai(
            "openai/gpt-4o-mini",
            "gpt-4o-mini",
            "OpenAI GPT-4o mini",
            &["gpt-4o-mini", "openai:gpt-4o-mini"],
            128_000,
            Some(16_384),
            true,
            true,
        ),
        openai(
            "openai/o4-mini",
            "o4-mini",
            "OpenAI o4-mini",
            &["o4-mini", "openai:o4-mini"],
            200_000,
            Some(100_000),
            true,
            true,
        ),
        openai(
            "openai/o3",
            "o3",
            "OpenAI o3",
            &["o3", "openai:o3"],
            200_000,
            Some(100_000),
            true,
            true,
        ),
        openai(
            "openai/o3-mini",
            "o3-mini",
            "OpenAI o3-mini",
            &["o3-mini", "openai:o3-mini"],
            200_000,
            Some(100_000),
            true,
            false,
        ),
        openai(
            "openai/o1",
            "o1",
            "OpenAI o1",
            &["o1", "openai:o1"],
            200_000,
            Some(100_000),
            true,
            true,
        ),
        openai(
            "openai/o1-mini",
            "o1-mini",
            "OpenAI o1-mini",
            &["o1-mini", "openai:o1-mini"],
            128_000,
            Some(65_536),
            true,
            false,
        ),
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
        None => format!(
            "{} ({} ctx)",
            entry.display_name,
            format_tokens(entry.context_window)
        ),
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
    fn resolves_provider_model_ids() {
        assert_eq!(
            provider_model_id_for_handle("openai/gpt-5.5"),
            Some("gpt-5.5")
        );
        assert_eq!(provider_model_id_for_handle("gpt-5.5"), Some("gpt-5.5"));
        assert_eq!(provider_model_id_for_handle("unknown-model"), None);
    }

    #[test]
    fn reports_gateway_compatibility() {
        let report = gateway_compatibility_report(["openai/gpt-4.1", "gpt-5", "openai/new-model"]);
        assert!(report
            .metadata_not_available
            .iter()
            .any(|h| h == "openai/gpt-4o"));
        assert!(report
            .available_without_metadata
            .iter()
            .any(|h| h == "openai/new-model"));
        assert!(report.available_with_metadata.iter().any(|h| h == "gpt-5"));
    }

    #[test]
    fn enriches_available_handle_with_metadata() {
        let option = model_option_for_available_handle("gpt-4.1", None, None, None);
        assert_eq!(option.handle, "gpt-4.1");
        assert!(option.label.contains("OpenAI GPT-4.1"));
        assert_eq!(option.context_window, Some(1_047_576));
    }

    #[test]
    fn catalog_responses_api_support_records_and_reads_by_canonical_key() {
        // Unique synthetic handles avoid colliding with other tests' global state.
        record_catalog_responses_api_support("vendor/resp-yes", Some(true));
        record_catalog_responses_api_support("vendor/resp-no", Some(false));

        assert_eq!(
            catalog_supports_responses_api("vendor/resp-yes"),
            Some(true)
        );
        assert_eq!(catalog_supports_responses_api("vendor/resp-no"), Some(false));
        assert_eq!(catalog_supports_responses_api("vendor/unobserved"), None);

        // `None` (catalog stated nothing) must not overwrite a prior record.
        record_catalog_responses_api_support("vendor/resp-yes", None);
        assert_eq!(
            catalog_supports_responses_api("vendor/resp-yes"),
            Some(true)
        );
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
