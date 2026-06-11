//! Den-native profile runtime registry ([ADR-0035](../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).

use std::str::FromStr;

use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        bears::{db as bears_db, model::BearProfile},
        runtime_contracts::{RoleProfileRegistry, RoleRuntimeBinding},
    },
    errors::CustomError,
};

pub struct DenNativeProfileRegistry<'a> {
    pool: &'a PgPool,
    config: &'a Config,
}

impl<'a> DenNativeProfileRegistry<'a> {
    pub fn new(pool: &'a PgPool, config: &'a Config) -> Self {
        Self { pool, config }
    }

    pub async fn resolve_binding(
        &self,
        bear_id: Uuid,
        profile: BearProfile,
    ) -> Result<Option<RoleRuntimeBinding>, CustomError> {
        if self.config.uses_native_agent_runtime() {
            let binding_id = bears_db::profile_binding_id(self.pool, bear_id, profile)
                .await?
                .filter(|id| !id.trim().is_empty())
                .unwrap_or_else(|| format!("den-native:{bear_id}:{}", profile.as_str()));
            return Ok(Some(RoleRuntimeBinding {
                binding_id,
                compatibility_backend: Some("runtime:native".to_string()),
            }));
        }
        Ok(bears_db::profile_binding_id(self.pool, bear_id, profile)
            .await?
            .map(|binding_id| RoleRuntimeBinding {
                binding_id,
                compatibility_backend: Some("runtime:letta".to_string()),
            }))
    }
}

#[allow(async_fn_in_trait)]
impl RoleProfileRegistry for DenNativeProfileRegistry<'_> {
    async fn resolve_compatibility_binding(
        &self,
        bear_id: Uuid,
        profile: &str,
    ) -> Result<Option<RoleRuntimeBinding>, CustomError> {
        let profile = BearProfile::from_str(profile).map_err(|_| {
            CustomError::ValidationError(format!("unknown bear profile: {profile}"))
        })?;
        self.resolve_binding(bear_id, profile).await
    }
}
