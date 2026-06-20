//! `BearProfile`: the operational profile vocabulary (ADR-0036/ADR-0039).
//!
//! This is a foundational closed-set enum shared across den crates (runtime,
//! docket, tools, memory, web/api), so it lives in `den-core`. The original
//! home was `core::bears::model`, which now re-exports it for backward
//! compatibility.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::ids::BearId;

/// Operational profile a Bear runs under (not a membership role).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BearProfile {
    Chat,
    Pair,
    Curate,
    Work,
    Watch,
}

impl BearProfile {
    pub const ALL: [Self; 5] = [
        Self::Chat,
        Self::Pair,
        Self::Curate,
        Self::Work,
        Self::Watch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Pair => "pair",
            Self::Curate => "curate",
            Self::Work => "work",
            Self::Watch => "watch",
        }
    }

    pub fn is_harness_backed(self) -> bool {
        matches!(self, Self::Chat | Self::Work)
    }

    pub fn runtime_family(self) -> &'static str {
        if self.is_harness_backed() {
            "native_harness_backed"
        } else {
            "native_api_direct"
        }
    }

    pub fn tags_for_bear(self, bear_id: BearId) -> Vec<String> {
        vec![
            format!("bear:{bear_id}"),
            format!("profile:{}", self.as_str()),
            format!("bear:{bear_id}:profile:{}", self.as_str()),
            "git-memory-enabled".to_string(),
        ]
    }
}

impl fmt::Display for BearProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BearProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "chat" => Ok(Self::Chat),
            "pair" => Ok(Self::Pair),
            "curate" => Ok(Self::Curate),
            "work" => Ok(Self::Work),
            "watch" => Ok(Self::Watch),
            other => Err(format!("unknown bear profile: {other}")),
        }
    }
}
