use serde::{Deserialize, Serialize};

/// Composable strategy knobs over the single tool-calling step primitive ([ADR-0033]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyProfile {
    pub plan: bool,
    pub reflect_on_fail: bool,
    pub critique: bool,
    pub fanout_n: u8,
}

impl StrategyProfile {
    pub fn plain_react() -> Self {
        Self {
            plan: false,
            reflect_on_fail: false,
            critique: false,
            fanout_n: 0,
        }
    }
}

impl Default for StrategyProfile {
    fn default() -> Self {
        Self::plain_react()
    }
}
