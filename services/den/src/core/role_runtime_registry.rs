//! Den-native role runtime profiles ([ADR-0035](../../docs/decisions/adr-0035-den-native-in-process-agent-runtime.md)).

use std::str::FromStr;

use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        bears::{db as bears_db, model::BearAgentRole},
        runtime_contracts::{RoleProfileRegistry, RoleRuntimeBinding},
    },
    errors::CustomError,
};

pub struct DenNativeRoleProfileRegistry<'a> {
    pool: &'a PgPool,
    config: &'a Config,
}

impl<'a> DenNativeRoleProfileRegistry<'a> {
    pub fn new(pool: &'a PgPool, config: &'a Config) -> Self {
        Self { pool, config }
    }

    pub async fn resolve_binding(
        &self,
        bear_id: Uuid,
        role: BearAgentRole,
    ) -> Result<Option<RoleRuntimeBinding>, CustomError> {
        if self.config.uses_native_agent_runtime() {
            let binding_id = bears_db::role_runtime_binding_id(self.pool, bear_id, role)
                .await?
                .filter(|id| !id.trim().is_empty())
                .unwrap_or_else(|| format!("den-native:{bear_id}:{}", role.as_str()));
            return Ok(Some(RoleRuntimeBinding {
                binding_id,
                compatibility_backend: Some("runtime:native".to_string()),
            }));
        }
        Ok(bears_db::role_runtime_binding_id(self.pool, bear_id, role)
            .await?
            .map(|binding_id| RoleRuntimeBinding {
                binding_id,
                compatibility_backend: Some("runtime:letta".to_string()),
            }))
    }
}

#[allow(async_fn_in_trait)]
impl RoleProfileRegistry for DenNativeRoleProfileRegistry<'_> {
    async fn resolve_compatibility_binding(
        &self,
        bear_id: Uuid,
        role: &str,
    ) -> Result<Option<RoleRuntimeBinding>, CustomError> {
        let role = BearAgentRole::from_str(role).map_err(|_| {
            CustomError::ValidationError(format!("unknown bear role: {role}"))
        })?;
        self.resolve_binding(bear_id, role).await
    }
}
