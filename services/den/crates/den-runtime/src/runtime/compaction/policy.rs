use den_core::profile::BearProfile;

use super::RuntimeCompactionPolicy;

/// Runtime compaction rollout mode (`COMPACTION_MODE` env).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionMode {
    /// Compaction evaluation disabled.
    Off,
    /// Evaluate policy and record events; do not mutate prompt assembly.
    Observe,
    /// Evaluate, record events, persist artifacts, and bound prompt transcript.
    Active,
}

impl CompactionMode {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "disabled" | "false" | "0" => Self::Off,
            "active" | "on" | "true" | "1" => Self::Active,
            _ => Self::Observe,
        }
    }
}

/// Per-profile compaction policy defaults.
pub fn compaction_policy_for_profile(profile: BearProfile) -> RuntimeCompactionPolicy {
    match profile {
        BearProfile::Pair => RuntimeCompactionPolicy {
            policy_version: "pair-v1".into(),
            protected_recent_group_count: 4,
            max_groups_before_compaction: 8,
            max_transcript_chars: 24_000,
        },
        BearProfile::Chat => RuntimeCompactionPolicy {
            policy_version: "chat-v1".into(),
            protected_recent_group_count: 3,
            max_groups_before_compaction: 10,
            max_transcript_chars: 16_000,
        },
        BearProfile::Work => RuntimeCompactionPolicy {
            policy_version: "work-v1".into(),
            protected_recent_group_count: 3,
            max_groups_before_compaction: 8,
            max_transcript_chars: 20_000,
        },
        BearProfile::Curate | BearProfile::Watch => RuntimeCompactionPolicy {
            policy_version: "background-v1".into(),
            protected_recent_group_count: 2,
            max_groups_before_compaction: 6,
            max_transcript_chars: 12_000,
        },
    }
}
