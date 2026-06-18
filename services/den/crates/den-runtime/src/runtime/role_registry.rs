//! Den-native profile runtime registry ([ADR-0035](../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).

use std::str::FromStr;

use sqlx::PgPool;
use uuid::Uuid;

use crate::runtime_contracts::{RoleProfileRegistry, RoleRuntimeBinding};
use den_core::config::Config;
use den_core::{BearProfile, DenError};

pub struct DenNativeProfileRegistry<'a> {
    pool: &'a PgPool,
}

impl<'a> DenNativeProfileRegistry<'a> {
    pub fn new(pool: &'a PgPool, _config: &'a Config) -> Self {
        Self { pool }
    }

    pub async fn resolve_binding(
        &self,
        bear_id: Uuid,
        profile: BearProfile,
    ) -> Result<Option<RoleRuntimeBinding>, DenError> {
        // Inline lookup of the bear↔profile binding id. This intentionally duplicates
        // the small `bears::db::profile_binding_id` query so den-runtime does not depend
        // on the `den`-crate `bears` subsystem (a `den-runtime → den` cycle); the bears
        // subsystem keeps its own copy for the edge call sites until it, too, lands here.
        let row: Option<(String,)> = sqlx::query_as(
            r"
            SELECT binding_id
            FROM bear_profile_bindings
            WHERE bear_id = $1 AND profile = $2
            ",
        )
        .bind(bear_id)
        .bind(profile.as_str())
        .fetch_optional(self.pool)
        .await?;
        let binding_id = row
            .map(|r| r.0)
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("den-native:{bear_id}:{}", profile.as_str()));
        Ok(Some(RoleRuntimeBinding {
            binding_id,
            compatibility_backend: Some("runtime:native".to_string()),
        }))
    }
}

#[allow(async_fn_in_trait)]
impl RoleProfileRegistry for DenNativeProfileRegistry<'_> {
    async fn resolve_compatibility_binding(
        &self,
        bear_id: Uuid,
        profile: &str,
    ) -> Result<Option<RoleRuntimeBinding>, DenError> {
        let profile = BearProfile::from_str(profile).map_err(|_| {
            DenError::ValidationError(format!("unknown bear profile: {profile}"))
        })?;
        self.resolve_binding(bear_id, profile).await
    }
}
