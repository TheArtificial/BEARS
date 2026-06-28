use den_core::tools::context::DenToolInvocationContext;
use den_core::tools::entity::{
    EntityBrowseArguments, EntityLinkMemoryArguments, EntityMergeArguments, EntityResolveArguments,
    EntitySplitArguments, EntityWriteAccessRuleArguments,
};
use den_runtime::bears::BearProfile;
use den_runtime::memory::store::{
    self as memory_store, descriptors, EntityHandleRow, EntityRow, EntityTrust, RelationClass,
    RelationRow, ResolutionState,
};
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

    pub(crate) async fn link_memory(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError> {
        let args: EntityLinkMemoryArguments = serde_json::from_value(arguments)?;
        let descriptor = descriptors::relation(&args.relation).ok_or_else(|| {
            DenError::ValidationError(format!("unknown entity relation: {}", args.relation))
        })?;
        if descriptor.class != RelationClass::Descriptive {
            return Err(DenError::Authorization(
                "entity_link_memory can only write descriptive relations".to_string(),
            ));
        }
        let store = self.ctx.stores.store_for_bear(context.bear_id).await?;
        let qualifiers = args.qualifiers.unwrap_or_else(|| json!({}));
        let relation = memory_store::append_relation(
            &store,
            &args.memory_id,
            &args.entity_id,
            &args.relation,
            &qualifiers,
            role.as_str(),
            Some(context.binding_id.as_str()),
            args.confidence.as_deref(),
        )
        .await?;
        Ok(json!({
            "ok": true,
            "bear_id": context.bear_id,
            "role": role.as_str(),
            "relation": relation,
        }))
    }

    pub(crate) async fn merge(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError> {
        if role != BearProfile::Curate {
            return Err(DenError::Authorization(
                "entity_merge is available only to curate".to_string(),
            ));
        }
        let args: EntityMergeArguments = serde_json::from_value(arguments)?;
        let store = self.ctx.stores.store_for_bear(context.bear_id).await?;
        let survivor =
            memory_store::merge_entities(&store, &args.survivor_entity_id, &args.loser_entity_id)
                .await?;
        Ok(json!({
            "ok": true,
            "bear_id": context.bear_id,
            "role": role.as_str(),
            "survivor": survivor,
            "loser_entity_id": args.loser_entity_id,
        }))
    }

    pub(crate) async fn split(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError> {
        if role != BearProfile::Curate {
            return Err(DenError::Authorization(
                "entity_split is available only to curate".to_string(),
            ));
        }
        let args: EntitySplitArguments = serde_json::from_value(arguments)?;
        if args.handle_ids_to_move.is_empty() {
            return Err(DenError::ValidationError(
                "handle_ids_to_move must not be empty".to_string(),
            ));
        }
        let resolution = match args.resolution.as_deref().unwrap_or("provisional") {
            "observed" => ResolutionState::Observed,
            "provisional" => ResolutionState::Provisional,
            "resolved" => ResolutionState::Resolved,
            "confirmed" => ResolutionState::Confirmed,
            other => {
                return Err(DenError::ValidationError(format!(
                    "unsupported resolution: {other}"
                )))
            }
        };
        let trust = match args.trust.as_deref().unwrap_or("inferred") {
            "inferred" => EntityTrust::Inferred,
            "asserted" => EntityTrust::Asserted,
            other => {
                return Err(DenError::ValidationError(format!(
                    "unsupported trust: {other}"
                )))
            }
        };
        let store = self.ctx.stores.store_for_bear(context.bear_id).await?;
        let entity = memory_store::split_entity(
            &store,
            &args.new_entity_type,
            args.display_name.as_deref(),
            &args.handle_ids_to_move,
            resolution,
            trust,
        )
        .await?;
        Ok(json!({
            "ok": true,
            "bear_id": context.bear_id,
            "role": role.as_str(),
            "entity": entity,
            "moved_handle_ids": args.handle_ids_to_move,
        }))
    }

    pub(crate) async fn write_access_rule(
        &self,
        context: &DenToolInvocationContext,
        role: BearProfile,
        arguments: Value,
    ) -> Result<Value, DenError> {
        if role != BearProfile::Curate {
            return Err(DenError::Authorization(
                "entity_write_access_rule is available only to curate".to_string(),
            ));
        }
        let args: EntityWriteAccessRuleArguments = serde_json::from_value(arguments)?;
        let descriptor = descriptors::relation(&args.relation).ok_or_else(|| {
            DenError::ValidationError(format!("unknown entity relation: {}", args.relation))
        })?;
        if descriptor.class != RelationClass::AccessBearing {
            return Err(DenError::ValidationError(
                "entity_write_access_rule can only write access-bearing relations".to_string(),
            ));
        }
        let store = self.ctx.stores.store_for_bear(context.bear_id).await?;
        let qualifiers = args.qualifiers.unwrap_or_else(|| json!({}));
        let relation = memory_store::append_relation(
            &store,
            &args.memory_id,
            &args.entity_id,
            &args.relation,
            &qualifiers,
            role.as_str(),
            Some(context.binding_id.as_str()),
            args.confidence.as_deref(),
        )
        .await?;
        Ok(json!({
            "ok": true,
            "bear_id": context.bear_id,
            "role": role.as_str(),
            "relation": relation,
        }))
    }
}
