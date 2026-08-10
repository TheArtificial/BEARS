use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextBudgetEstimatePrecision {
    Exact,
    Approximate,
    /// Approximate estimate corrected by an observed per-model chars→tokens
    /// ratio mirrored from Bifrost usage (ADR-0047 §7).
    CalibratedApproximate,
}

/// The chars→tokens ratio applied to this report's approximate estimates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBudgetCalibrationReport {
    /// `model_registry` when an observed per-model ratio was applied;
    /// `default` for the uncalibrated chars/4 heuristic.
    pub source: String,
    /// Applied ratio scaled to tokens per million characters (250_000 = chars/4).
    pub tokens_per_million_chars: u32,
    /// Observed samples backing the ratio (0 for the default heuristic).
    pub sample_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBudgetComponentReport {
    pub key: String,
    pub label: String,
    pub estimated_tokens: u32,
    pub estimated_characters: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBudgetReport {
    pub model: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub reserved_output_tokens: u32,
    pub estimated_input_tokens: u32,
    pub estimated_total_tokens: u32,
    pub estimate_precision: ContextBudgetEstimatePrecision,
    pub near_budget: bool,
    pub over_budget: bool,
    /// Ratio provenance for the approximate estimates; absent on reports
    /// persisted before calibration landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<ContextBudgetCalibrationReport>,
    pub components: Vec<ContextBudgetComponentReport>,
}
