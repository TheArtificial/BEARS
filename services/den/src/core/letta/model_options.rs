//! Pure option/value types retained for native code (Bifrost model catalog, bear create UI,
//! tool-policy filtering). These were originally shaped from Letta `GET /v1/models/` and
//! `GET /v1/tools/` responses; they survive the removal of the self-hosted Letta HTTP client.

/// One selectable LLM model row for `<select>` options.
///
/// Constructed from Bifrost model metadata (see [`crate::core::llm::bifrost`]).
#[derive(Debug, Clone, serde::Serialize)]
pub struct LettaModelOption {
    /// Value for the agent `model` / Den `default_model` (e.g. `gpt-4o`).
    pub handle: String,
    /// Human-facing label for the operator UI.
    pub label: String,
    /// Provider/model context window from Bifrost metadata when available.
    pub context_window: Option<u32>,
    /// Maximum output tokens from Bifrost metadata when available.
    pub max_output_tokens: Option<u32>,
}

/// One selectable tool row for multi-select (`tool_ids` on create/patch).
#[derive(Debug, Clone, serde::Serialize)]
pub struct LettaToolOption {
    pub id: String,
    pub label: String,
}
