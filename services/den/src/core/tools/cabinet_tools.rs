//! Model-facing Cabinet tools: search/read/create/update over the Phase 1
//! facade in `den_service::cabinet`.
//!
//! These run on the root-crate dispatch path (like workflow tools), so
//! `authorize_den_tool` does not run here: each executor enforces its own
//! role policy, and the facade enforces the Bear-level `cabinet_enabled`
//! gate and contract rules server-side.

use den_cabinet::{
    ActorScope, CabinetItemRef, CabinetVersionRef, CreateItemRequest, ItemKind, Lifecycle,
    NewSourceLink, ReadRequest, SearchFilters, SearchRequest, UpdateItemRequest,
};
use den_core::ids::{BearId, ConversationId};
use den_core::tools::constants::{
    DEN_CABINET_CREATE, DEN_CABINET_READ, DEN_CABINET_SEARCH, DEN_CABINET_UPDATE,
};
use den_core::tools::context::DenToolInvocationContext;
use den_core::BearProfile;
use den_http::errors::CustomError;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

pub(crate) fn is_cabinet_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        DEN_CABINET_SEARCH | DEN_CABINET_READ | DEN_CABINET_CREATE | DEN_CABINET_UPDATE
    )
}

fn cabinet_error(error: den_cabinet::CabinetError) -> CustomError {
    CustomError::from(den_core::DenError::from(error))
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, CustomError> {
    serde_json::from_value(arguments)
        .map_err(|error| CustomError::ValidationError(error.to_string()))
}

fn actor_scope(context: &DenToolInvocationContext, role: BearProfile) -> ActorScope {
    let mut scope = ActorScope::bear(BearId::new(context.bear_id), role);
    if !context.conversation_id.is_empty() {
        scope.conversation_id = Some(ConversationId(context.conversation_id.clone()));
    }
    scope.run_id = context.work_run_id.map(|run_id| run_id.to_string());
    scope
}

fn require_write_role(role: BearProfile) -> Result<(), CustomError> {
    if matches!(
        role,
        BearProfile::Chat | BearProfile::Pair | BearProfile::Curate
    ) {
        Ok(())
    } else {
        Err(CustomError::Authorization(format!(
            "cabinet writes are not available to the {role} stance"
        )))
    }
}

pub(crate) async fn invoke_cabinet_tool(
    pool: &PgPool,
    tool_name: &str,
    arguments: Value,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    let role = context.profile.unwrap_or(BearProfile::Pair);
    match tool_name {
        DEN_CABINET_SEARCH => cabinet_search(pool, context, role, arguments).await,
        DEN_CABINET_READ => cabinet_read(pool, context, role, arguments).await,
        DEN_CABINET_CREATE => cabinet_create(pool, context, role, arguments).await,
        DEN_CABINET_UPDATE => cabinet_update(pool, context, role, arguments).await,
        other => Err(CustomError::NotFound(format!(
            "unknown cabinet tool: {other}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct CabinetSearchArguments {
    query: String,
    #[serde(default)]
    lifecycle: Option<Lifecycle>,
}

async fn cabinet_search(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: CabinetSearchArguments = parse_arguments(arguments)?;
    let items = den_service::cabinet::search(
        pool,
        SearchRequest {
            scope: actor_scope(context, role),
            query: args.query,
            filters: SearchFilters {
                lifecycle: args.lifecycle,
                ..SearchFilters::default()
            },
        },
    )
    .await
    .map_err(cabinet_error)?;
    Ok(json!({ "domain": "cabinet", "items": items }))
}

#[derive(Debug, Deserialize)]
struct CabinetReadArguments {
    cabinet_ref: String,
    #[serde(default)]
    version_ref: Option<String>,
}

async fn cabinet_read(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    let args: CabinetReadArguments = parse_arguments(arguments)?;
    let view = den_service::cabinet::read(
        pool,
        ReadRequest {
            scope: actor_scope(context, role),
            cabinet_ref: CabinetItemRef::parse(&args.cabinet_ref)
                .map_err(|error| CustomError::ValidationError(error.to_string()))?,
            version_ref: args
                .version_ref
                .as_deref()
                .map(CabinetVersionRef::parse)
                .transpose()
                .map_err(|error| CustomError::ValidationError(error.to_string()))?,
        },
    )
    .await
    .map_err(cabinet_error)?;
    Ok(
        json!({ "domain": "cabinet", "item": view.item, "version": view.version, "sources": view.sources }),
    )
}

#[derive(Debug, Deserialize)]
struct CabinetCreateArguments {
    title: String,
    content: String,
    #[serde(default)]
    source_links: Vec<NewSourceLink>,
}

async fn cabinet_create(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    require_write_role(role)?;
    let args: CabinetCreateArguments = parse_arguments(arguments)?;
    let view = den_service::cabinet::create_item(
        pool,
        CreateItemRequest {
            scope: actor_scope(context, role),
            kind: ItemKind::Document,
            title: args.title,
            content: args.content,
            collection_ref: None,
            mission_ref: None,
            source_links: args.source_links,
        },
    )
    .await
    .map_err(cabinet_error)?;
    Ok(
        json!({ "domain": "cabinet", "item": view.item, "version": view.version, "sources": view.sources }),
    )
}

#[derive(Debug, Deserialize)]
struct CabinetUpdateArguments {
    cabinet_ref: String,
    content: String,
    base_version: String,
    #[serde(default)]
    title: Option<String>,
}

async fn cabinet_update(
    pool: &PgPool,
    context: &DenToolInvocationContext,
    role: BearProfile,
    arguments: Value,
) -> Result<Value, CustomError> {
    require_write_role(role)?;
    let args: CabinetUpdateArguments = parse_arguments(arguments)?;
    let view = den_service::cabinet::update_item(
        pool,
        UpdateItemRequest {
            scope: actor_scope(context, role),
            cabinet_ref: CabinetItemRef::parse(&args.cabinet_ref)
                .map_err(|error| CustomError::ValidationError(error.to_string()))?,
            content: args.content,
            base_version: CabinetVersionRef::parse(&args.base_version)
                .map_err(|error| CustomError::ValidationError(error.to_string()))?,
            title: args.title,
        },
    )
    .await
    .map_err(cabinet_error)?;
    Ok(
        json!({ "domain": "cabinet", "item": view.item, "version": view.version, "sources": view.sources }),
    )
}
