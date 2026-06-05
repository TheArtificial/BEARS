use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use crate::{
    api::service::ApiState,
    core::{
        acp_runtime::{require_pair_runtime_binding, verify_acp_conversation_access},
        archived_conversations,
        bears::db as bears_db,
        conversation_persistence::{
            count_visible_messages, ensure_conversation_for_external_id, insert_message_if_absent,
            list_conversations_for_bear, list_messages_page,
        },
        runtime_compaction_store::{list_runtime_compaction_events, record_runtime_compaction_event},
    },
    errors::CustomError,
};

use crate::api::acp::{
    history::{
        map_acp_history_page, map_canonical_history_page, map_compaction_status_for_history,
        runtime_compaction_event_for_history, runtime_messages_for_persistence,
    },
    normalize_acp_conversation_id,
    responses::acp_error_response,
    AcpConversationHistoryQuery, AcpConversationHistoryResponse, AcpConversationRow,
    AcpConversationsQuery, AcpConversationsResponse,
};

use super::auth::authenticate_acp_code_token;

pub(in crate::api::acp) async fn conversations(
    State(state): State<ApiState>,
    Path(slug): Path<String>,
    Query(query): Query<AcpConversationsQuery>,
    headers: HeaderMap,
) -> Response {
    let request_id = Uuid::new_v4();
    match conversations_inner(state, slug, query, headers).await {
        Ok(response) => response,
        Err(err) => acp_error_response(err, request_id),
    }
}

pub(super) async fn conversations_inner(
    state: ApiState,
    slug: String,
    query: AcpConversationsQuery,
    headers: HeaderMap,
) -> Result<Response, CustomError> {
    let user_id = authenticate_acp_code_token(&state, &headers, &slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, slug.trim())
        .await?
        .ok_or_else(|| {
            CustomError::NotFound("bear not found or you do not have access".to_string())
        })?;

    let archived_ids = archived_conversations::list_for_bear(&state.sqlx_pool, bear.id).await?;
    let canonical = list_conversations_for_bear(&state.sqlx_pool, bear.id, 200).await?;
    let mut conversations: Vec<AcpConversationRow> = canonical
        .into_iter()
        .filter_map(|row| {
            let id = row.external_conversation_id?;
            let archived = archived_ids.contains(&id);
            if archived && !query.include_archived {
                return None;
            }
            Some(AcpConversationRow {
                id,
                title: row.current_title.unwrap_or_else(|| "Main chat".to_string()),
                last_message_at: Some(row.updated_at.to_string()),
                archived,
            })
        })
        .collect();

    if conversations.is_empty() {
        conversations.push(AcpConversationRow {
            id: "default".to_string(),
            title: "Main chat".to_string(),
            last_message_at: None,
            archived: false,
        });
    } else if !conversations.iter().any(|row| row.id == "default") && !query.include_archived {
        conversations.push(AcpConversationRow {
            id: "default".to_string(),
            title: "Main chat".to_string(),
            last_message_at: None,
            archived: false,
        });
    }

    conversations.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at).then_with(|| a.id.cmp(&b.id)));
    Ok(Json(AcpConversationsResponse { conversations }).into_response())
}

pub(in crate::api::acp) async fn conversation_history(
    State(state): State<ApiState>,
    Path((slug, conversation_id)): Path<(String, String)>,
    Query(query): Query<AcpConversationHistoryQuery>,
    headers: HeaderMap,
) -> Response {
    let request_id = Uuid::new_v4();
    match conversation_history_inner(state, slug, conversation_id, query, headers).await {
        Ok(response) => response,
        Err(err) => acp_error_response(err, request_id),
    }
}

pub(super) async fn conversation_history_inner(
    state: ApiState,
    slug: String,
    conversation_id: String,
    query: AcpConversationHistoryQuery,
    headers: HeaderMap,
) -> Result<Response, CustomError> {
    let user_id = authenticate_acp_code_token(&state, &headers, &slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, slug.trim())
        .await?
        .ok_or_else(|| {
            CustomError::NotFound("bear not found or you do not have access".to_string())
        })?;
    if !state.letta.is_enabled() {
        return Ok(Json(AcpConversationHistoryResponse {
            messages: vec![],
            has_more: false,
            next_before: None,
            compaction: None,
            compaction_history: vec![],
        })
        .into_response());
    }
    let runtime_binding =
        require_pair_runtime_binding(&state.sqlx_pool, state.letta.as_ref(), &bear).await?;
    let agent_id = runtime_binding.binding_id.clone();
    let conv_id = normalize_acp_conversation_id(Some(&conversation_id))?;
    if conv_id.starts_with("new-") {
        return Err(CustomError::ValidationError(
            "history is only available for default or saved conv- conversations".to_string(),
        ));
    }
    if conv_id.starts_with("conv-") {
        verify_acp_conversation_access(
            &state.sqlx_pool,
            bear.id,
            state.letta.as_ref(),
            &runtime_binding,
            &conv_id,
        )
        .await?;
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let before = query
        .before
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let binding_for_conv = if conv_id == "default" {
        Some(agent_id.as_str())
    } else {
        None
    };
    let before_sequence_no = before.and_then(|value| value.parse::<i64>().ok());
    let canonical_conversation = ensure_conversation_for_external_id(
        &state.sqlx_pool,
        bear.id,
        Some(user_id),
        &conv_id,
        None,
        None,
    )
    .await?;
    let canonical_rows = list_messages_page(
        &state.sqlx_pool,
        canonical_conversation.id,
        before_sequence_no,
        i64::from(limit),
    )
    .await?;
    let canonical_visible_count = count_visible_messages(&state.sqlx_pool, canonical_conversation.id).await?;
    if canonical_visible_count > 0 {
        let (messages, has_more, next_before) = map_canonical_history_page(&canonical_rows, limit);
        let compaction_history = list_runtime_compaction_events(&state.sqlx_pool, &conv_id, 10)
            .await
            .unwrap_or_default();
        return Ok(Json(AcpConversationHistoryResponse {
            messages,
            has_more,
            next_before,
            compaction: None,
            compaction_history,
        })
        .into_response());
    }
    let body = state
        .letta
        .list_conversation_messages(&conv_id, binding_for_conv, limit, before, false)
        .await?;
    for (index, raw_message) in runtime_messages_for_persistence(&body).iter().enumerate() {
        let inner = raw_message.get("contents").unwrap_or(raw_message);
        let message_type = inner
            .get("message_type")
            .and_then(|v| v.as_str())
            .unwrap_or("system_message");
        let normalized_message_type = match message_type {
            "user_message" => "user",
            "assistant_message" => "assistant",
            _ => "system",
        };
        let role = inner
            .get("role")
            .and_then(|v| v.as_str())
            .or_else(|| raw_message.get("role").and_then(|v| v.as_str()));
        let content_text = inner
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| {
                inner
                    .get("content")
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let provider_message_id = raw_message
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| inner.get("id").and_then(|v| v.as_str()));
        let created_at = raw_message
            .get("date")
            .or_else(|| raw_message.get("created_at"))
            .and_then(|v| v.as_str());
        insert_message_if_absent(
            &state.sqlx_pool,
            canonical_conversation.id,
            index as i64,
            normalized_message_type,
            role,
            "default",
            &content_text,
            inner.clone(),
            provider_message_id,
            created_at,
        )
        .await?;
    }
    let (messages, has_more, next_before) = map_acp_history_page(&body, limit);
    let event = runtime_compaction_event_for_history(
        &conv_id,
        &body,
        crate::core::runtime_conversations::RuntimeCompactionTriggerKind::SemanticGroupCount,
    );
    let _ = record_runtime_compaction_event(&state.sqlx_pool, &event).await;
    let compaction = Some(map_compaction_status_for_history(&conv_id, &body));
    let compaction_history = list_runtime_compaction_events(&state.sqlx_pool, &conv_id, 10)
        .await
        .unwrap_or_default();
    Ok(Json(AcpConversationHistoryResponse {
        messages,
        has_more,
        next_before,
        compaction,
        compaction_history,
    })
    .into_response())
}
