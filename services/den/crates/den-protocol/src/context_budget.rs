use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextBudgetEstimatePrecision {
    Exact,
    Approximate,
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
    pub components: Vec<ContextBudgetComponentReport>,
}
