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
    /// Deprecated legacy provider `agent_type`; ignored by Den-native provisioning.
    pub letta_agent_type: Option<String>,
    /// Deprecated legacy provider tool ids; ignored by Den-native provisioning.
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

// `BearStance` now lives in the `den-core` foundation crate (shared by runtime,
// docket, tools, memory, web/api). `BearProfile` is re-exported as a deprecated
// compatibility alias while call sites migrate to stance terminology.
pub use den_core::{BearProfile, BearStance};

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
mod tests;

