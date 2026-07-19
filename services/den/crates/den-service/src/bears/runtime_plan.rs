//! Default [`runtime_plan`](super::model::Bear) JSON (versioned snapshot).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const RUNTIME_PLAN_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePlan {
    #[serde(default = "default_runtime_plan_version")]
    pub version: u32,
    #[serde(default)]
    pub memory: RuntimePlanMemory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePlanMemory {
    #[serde(default)]
    pub git_remote: Option<String>,
    #[serde(default = "default_memory_git_ref")]
    pub git_ref: String,
    #[serde(default = "default_memory_seed_template")]
    pub seed_template: String,
}

impl Default for RuntimePlan {
    fn default() -> Self {
        Self {
            version: RUNTIME_PLAN_VERSION,
            memory: RuntimePlanMemory::default(),
        }
    }
}

impl Default for RuntimePlanMemory {
    fn default() -> Self {
        Self {
            git_remote: None,
            git_ref: default_memory_git_ref(),
            seed_template: default_memory_seed_template(),
        }
    }
}

fn default_runtime_plan_version() -> u32 {
    RUNTIME_PLAN_VERSION
}

fn default_memory_git_ref() -> String {
    "main".to_string()
}

fn default_memory_seed_template() -> String {
    "default".to_string()
}

/// Default plan when `bears.runtime_plan` is NULL (new bears, rollout).
pub fn default_runtime_plan() -> Value {
    serde_json::to_value(RuntimePlan::default()).expect("RuntimePlan serializes to JSON")
}

/// Merge DB column with defaults so callers receive a full v1 object.
pub fn effective_runtime_plan(stored: Option<&Value>) -> serde_json::Result<Value> {
    let plan = match stored {
        Some(stored) => serde_json::from_value::<RuntimePlan>(stored.clone())?,
        None => RuntimePlan::default(),
    };
    serde_json::to_value(plan)
}

#[cfg(test)]
mod tests;
