use serde::Deserialize;
use serde_json::Value;

use crate::tools::context::DenToolInvocationContext;
use crate::{BearProfile, DenError};

#[derive(Debug, Deserialize)]
pub struct EntityBrowseArguments {
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct EntityResolveArguments {
    pub entity_id: String,
    #[serde(default)]
    pub include_relations: Option<bool>,
    #[serde(default)]
    pub include_handles: Option<bool>,
}

#[allow(async_fn_in_trait)]
pub trait EntityOps: Send + Sync {
    async fn browse_entities(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError>;

    async fn resolve_entity(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError>;
}

pub async fn entity_browse(
    ops: &impl EntityOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    ops.browse_entities(context, role, arguments).await
}

pub async fn entity_resolve(
    ops: &impl EntityOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    ops.resolve_entity(context, role, arguments).await
}
