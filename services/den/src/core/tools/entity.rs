use den_core::tools::context::DenToolInvocationContext;
use den_core::tools::entity::{EntityBrowseArguments, EntityResolveArguments};
use den_runtime::bears::BearProfile;
use den_runtime::memory::store::{self as memory_store, EntityHandleRow, EntityRow, RelationRow};
use serde_json::{json, Value};

use crate::core::tools::context::DenToolContext;
use crate::errors::DenError;

pub(crate) struct DenEntityOps<'a> {
    ctx: &'a DenToolContext<'a>,
}

impl<'a> DenEntityOps<'a> {
    pub(crate) fn new(ctx: &'a DenToolContext<'a>) -> Self {
        Self { ctx }
    }

    pub(crate) async fn browse(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError> {
        let args: EntityBrowseArguments = serde_json::from_value(arguments)?;
        let limit = args.limit.unwrap_or(25).clamp(1, 100);
        let store = self.ctx.stores.store_for_bear(context.bear_id).await?;
        let entities: Vec<EntityRow> =
            memory_store::list_entities(&store, args.entity_type.as_deref(), limit).await?;
        Ok(json!({
            "ok": true,
            "bear_id": context.bear_id,
            "role": role.as_str(),
            "entity_type": args.entity_type,
            "entities": entities,
        }))
    }

    pub(crate) async fn resolve(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError> {
        let args: EntityResolveArguments = serde_json::from_value(arguments)?;
        let store = self.ctx.stores.store_for_bear(context.bear_id).await?;
        let Some(entity) = memory_store::resolve_live_entity(&store, &args.entity_id).await? else {
            return Ok(json!({
                "ok": false,
                "bear_id": context.bear_id,
                "role": role.as_str(),
                "entity_id": args.entity_id,
                "message": "entity not found",
            }));
        };
        let handles = if args.include_handles.unwrap_or(true) {
            let rows: Vec<EntityHandleRow> =
                memory_store::list_handles(&store, &entity.entity_id).await?;
            json!(rows)
        } else {
            Value::Null
        };
        let relations = if args.include_relations.unwrap_or(false) {
            let rows: Vec<RelationRow> =
                memory_store::list_relations_for_entity(&store, &entity.entity_id, 50).await?;
            json!(rows)
        } else {
            Value::Null
        };
        Ok(json!({
            "ok": true,
            "bear_id": context.bear_id,
            "role": role.as_str(),
            "entity": entity,
            "handles": handles,
            "relations": relations,
        }))
    }
}
