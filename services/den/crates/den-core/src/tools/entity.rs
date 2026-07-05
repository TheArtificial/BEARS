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

#[derive(Debug, Deserialize)]
pub struct EntityLinkMemoryArguments {
    pub memory_id: String,
    pub entity_id: String,
    pub relation: String,
    #[serde(default)]
    pub qualifiers: Option<Value>,
    #[serde(default)]
    pub confidence: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EntityMergeArguments {
    pub survivor_entity_id: String,
    pub loser_entity_id: String,
}

#[derive(Debug, Deserialize)]
pub struct EntitySplitArguments {
    pub new_entity_type: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub handle_ids_to_move: Vec<String>,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub trust: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EntityWriteAccessRuleArguments {
    pub memory_id: String,
    pub entity_id: String,
    pub relation: String,
    #[serde(default)]
    pub qualifiers: Option<Value>,
    #[serde(default)]
    pub confidence: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EntityWriteAnchorArguments {
    pub entity_id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub salience: Option<String>,
    #[serde(default)]
    pub supersedes_memory_id: Option<String>,
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

    async fn link_memory_entity(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError>;

    async fn merge_entities_tool(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError>;

    async fn split_entity_tool(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError>;

    async fn write_entity_access_rule(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError>;

    async fn write_entity_anchor(
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

pub async fn entity_link_memory(
    ops: &impl EntityOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    ops.link_memory_entity(context, role, arguments).await
}

pub async fn entity_merge(
    ops: &impl EntityOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    ops.merge_entities_tool(context, role, arguments).await
}

pub async fn entity_split(
    ops: &impl EntityOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    ops.split_entity_tool(context, role, arguments).await
}
pub async fn entity_write_access_rule(
    ops: &impl EntityOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    ops.write_entity_access_rule(context, role, arguments).await
}

pub async fn entity_write_anchor(
    ops: &impl EntityOps,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, DenError> {
    ops.write_entity_anchor(context, role, arguments).await
}
