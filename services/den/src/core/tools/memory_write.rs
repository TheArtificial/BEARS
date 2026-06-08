use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{
    config::Config,
    core::{
        bears::BearAgentRole,
        memory::{tools as sqlite_memory, MemoryStoreManager},
        memory_manager_head::MemfsWriteRoleMemoryEntryRequest,
        user,
        tools::{
            memfs::{memfs_http_client, write_role_memory_entry},
            session::DenToolInvocationContext,
            support::{
                clean_limited_strings, clean_optional, validate_bounded_text,
                validate_memory_write_entry_semantics, validate_optional_object,
            },
        },
    },
    errors::CustomError,
};

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryWriteEntryArguments {
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) refs: Option<Value>,
    #[serde(default)]
    pub(crate) lifecycle: Option<Value>,
    #[serde(default)]
    pub(crate) source: Option<Value>,
    #[serde(default)]
    pub(crate) content_class: Option<String>,
    #[serde(default)]
    pub(crate) domain: Option<String>,
    #[serde(default)]
    pub(crate) semantic_confirmation_token: Option<String>,
}

pub(crate) fn merge_memory_entry_source_with_human(
    source: Option<Value>,
    context: &DenToolInvocationContext,
    current_user: Option<&user::User>,
) -> Option<Value> {
    let mut source_obj = source
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    source_obj.insert(
        "human".to_string(),
        json!({
            "user_id": context.user_id,
            "username": current_user
                .map(|user| user.username.clone())
                .or_else(|| context.username.clone()),
            "display_name": current_user.map(|user| user.display_name.clone()),
            "membership_role": context.membership_role,
            "authenticated_by": "acp_token"
        }),
    );
    source_obj.insert(
        "session".to_string(),
        json!({
            "conversation_id": clean_optional(&context.conversation_id),
            "session_id": clean_optional(&context.session_id),
            "acp_session_id": context.acp_session_id,
            "conversation_selection": context.conversation_selection,
            "runtime_target": context.runtime_target,
            "request_id": context.request_id,
        }),
    );
    Some(Value::Object(source_obj))
}

pub(crate) fn source_acp_session_id(context: &DenToolInvocationContext) -> Option<String> {
    let is_acp = [
        context.channel.family.as_deref(),
        context.channel.client.as_deref(),
        context.channel.protocol.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_ascii_lowercase().contains("acp"));
    if is_acp {
        clean_optional(&context.session_id)
    } else {
        None
    }
}

pub(crate) async fn write_memory_entry(
    pool: &PgPool,
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearAgentRole::Pair {
        return Err(CustomError::Authorization(
            "den.memory.write_entry is currently available only to the pair role".to_string(),
        ));
    }
    let args: MemoryWriteEntryArguments = serde_json::from_value(arguments)?;
    let kind = validate_memory_write_entry_semantics(&args, context)?;
    let title = validate_bounded_text("title", &args.title, 1, 200)?;
    let body = validate_bounded_text("body", &args.body, 1, 50_000)?;
    let tags = clean_limited_strings(args.tags, 20, 80);
    validate_optional_object("refs", &args.refs)?;
    validate_optional_object("lifecycle", &args.lifecycle)?;
    validate_optional_object("source", &args.source)?;
    let current_user = user::user_by_id(pool, context.user_id).await.ok();
    let source = merge_memory_entry_source_with_human(args.source, context, current_user.as_ref());
    let authenticated_username = current_user
        .as_ref()
        .map(|user| user.username.clone())
        .or_else(|| context.username.clone());
    if config.uses_native_agent_runtime() {
        let stores = MemoryStoreManager::new(config);
        return sqlite_memory::sqlite_write_role_entry(
            &stores,
            config,
            context.bear_id,
            role.as_str(),
            &kind,
            &title,
            &body,
            &tags,
            source,
            authenticated_username,
        )
        .await;
    }
    let request = MemfsWriteRoleMemoryEntryRequest {
        kind,
        title,
        body,
        tags,
        refs: args.refs,
        lifecycle: args.lifecycle,
        source,
        author: authenticated_username,
        conversation_id: clean_optional(&context.conversation_id),
        session_id: source_acp_session_id(context).or_else(|| clean_optional(&context.session_id)),
        acp_session_id: context
            .acp_session_id
            .clone()
            .or_else(|| source_acp_session_id(context)),
        conversation_selection: context.conversation_selection.clone(),
        runtime_target: context.runtime_target.clone(),
        role_agent_id: Some(context.role_agent_id.clone()),
        agent_role: context.agent_role.map(|role| role.as_str().to_string()),
        request_id: context.request_id.clone(),
    };
    let http = memfs_http_client("MemFS memory entry client build failed")?;
    let response = write_role_memory_entry(
        &http,
        &config.letta_memfs_service_url,
        context.bear_id,
        role.as_str(),
        &request,
    )
    .await?;
    let Some(response) = response else {
        return Err(CustomError::System(
            "MemFS sidecar is not configured (set LETTA_MEMFS_SERVICE_URL)".to_string(),
        ));
    };
    Ok(json!({
        "bear_id": context.bear_id,
        "role": role.as_str(),
        "kind": response.kind,
        "entry_id": response.entry_id,
        "path": response.path,
        "commit": response.commit,
        "canonical_tip": response.canonical_tip,
        "view": response.view,
    }))
}
