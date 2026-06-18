//! Row shapes aligned with `migrations/*_phase1_*.sql`.
//! Use with `sqlx::query_as` when adding queries (keeps `SQLX_OFFLINE` builds predictable).

use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// Bear plus `user_bear.role` for the current user (`list_bears_for_user`).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BearWithMembership {
    #[sqlx(flatten)]
    pub bear: Bear,
    pub membership_role: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Bear {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub default_model: Option<String>,
    pub tools_enabled: Option<Json<serde_json::Value>>,
    /// Letta `agent_type` on create/patch when set (e.g. `memgpt_agent`, `letta_v1_agent`).
    pub letta_agent_type: Option<String>,
    /// Letta `tool_ids` on create/patch (JSON array of strings in Postgres).
    pub letta_tool_ids: Json<Vec<String>>,
    /// Optional BearRuntimePlan v1 JSON for codepool (memory git remote, seeds; extensible).
    pub runtime_plan: Option<Json<serde_json::Value>>,
    /// Optional profile-aware context composition profile.
    pub context_profile: Option<Json<serde_json::Value>>,
    /// Optional path to the Bear's bare MemFS repository.
    #[serde(default)]
    pub memfs_repo_path: Option<String>,
    /// Coarse canonical config version for profile provisioning/reconciliation.
    #[serde(default = "default_provisioning_version")]
    pub provisioning_version: i32,
    pub system_prompt: String,
    pub birthday: Option<time::Date>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

fn default_provisioning_version() -> i32 {
    1
}

// `BearProfile` now lives in the `den-core` foundation crate (shared by runtime,
// docket, tools, memory, web/api). Re-exported here so existing
// `crate::bears::{BearProfile, model::BearProfile}` paths keep compiling.
pub use den_core::BearProfile;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BearProfileBinding {
    pub bear_id: Uuid,
    pub profile: String,
    pub binding_id: String,
    /// Deprecated: legacy Letta agent id only; canonical identity is `binding_id`.
    pub letta_agent_id: Option<String>,
    pub provisioning_status: String,
    pub last_provisioned_version: i32,
    pub last_synced_at: Option<OffsetDateTime>,
    pub last_provisioning_error: Option<String>,
    pub config_hash: Option<Json<serde_json::Value>>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl BearProfileBinding {
    pub fn parsed_profile(&self) -> Result<BearProfile, String> {
        self.profile.parse()
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BearSkillManifestEntry {
    pub bear_id: Uuid,
    pub skill_name: String,
    pub skill_version: String,
    pub source: String,
    pub content_hash: String,
    pub applies_to_profiles: Vec<String>,
    pub installed_at: Option<OffsetDateTime>,
    pub last_verified_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BearSkillProposal {
    pub bear_id: Uuid,
    pub id: Uuid,
    pub proposed_by_agent_id: String,
    pub proposed_at: OffsetDateTime,
    pub skill_payload: Json<serde_json::Value>,
    pub status: String,
    pub reviewed_at: Option<OffsetDateTime>,
    pub rejection_reason: Option<String>,
    pub resulting_manifest_bear_id: Option<Uuid>,
    pub resulting_manifest_skill_name: Option<String>,
    pub resulting_manifest_skill_version: Option<String>,
    pub updated_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_parse_and_display_round_trip() {
        for profile in BearProfile::ALL {
            let parsed: BearProfile = profile.as_str().parse().expect("profile parses");
            assert_eq!(parsed, profile);
            assert_eq!(profile.to_string(), profile.as_str());
        }
        assert!("unknown".parse::<BearProfile>().is_err());
        assert!("talk".parse::<BearProfile>().is_err());
    }

    #[test]
    fn profile_runtime_family_matches_harness_design() {
        assert!(BearProfile::Chat.is_harness_backed());
        assert!(BearProfile::Work.is_harness_backed());
        assert!(!BearProfile::Pair.is_harness_backed());
        assert!(!BearProfile::Curate.is_harness_backed());
        assert!(!BearProfile::Watch.is_harness_backed());
        assert_eq!(BearProfile::Chat.runtime_family(), "letta_code_harness");
        assert_eq!(BearProfile::Work.runtime_family(), "letta_code_harness");
        assert_eq!(BearProfile::Pair.runtime_family(), "letta_api_direct");
    }

    #[test]
    fn profile_tags_include_bear_profile_and_git_memory() {
        let bear_id = Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap();
        let tags = BearProfile::Work.tags_for_bear(den_core::BearId::from(bear_id));
        assert!(tags.contains(&format!("bear:{bear_id}")));
        assert!(tags.contains(&"profile:work".to_string()));
        assert!(tags.contains(&format!("bear:{bear_id}:profile:work")));
        assert!(tags.contains(&"git-memory-enabled".to_string()));
    }
}
