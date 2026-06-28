use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use crate::service::DenState;
use den_http::errors::CustomError;
use den_service::{
    bears::db as bears_db,
    conversation::persistence::{
        count_visible_messages, ensure_conversation_for_external_id, list_conversations_for_bear,
        list_messages_page,
    },
};
use den_runtime::{
    runtime_compaction_store::{
        latest_compaction_artifact_for_conversation, list_runtime_compaction_events,
    },
};

use crate::acp::{
    history::map_canonical_history_page,
    normalize_acp_conversation_id,
    responses::acp_error_response,
    AcpConversationHistoryQuery, AcpConversationHistoryResponse, AcpConversationRow,
    AcpConversationsQuery, AcpConversationsResponse,
};

use super::auth::authenticate_acp_code_token;

pub(in crate::acp) async fn conversations(
    State(state): State<DenState>,
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
    state: DenState,
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

    let archived_ids = den_service::archived_conversations::list_for_bear(&state.sqlx_pool, bear.id).await?;
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

    let needs_default_row = conversations.is_empty()
        || (!conversations.iter().any(|row| row.id == "default") && !query.include_archived);
    if needs_default_row {
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

pub(in crate::acp) async fn conversation_history(
    State(state): State<DenState>,
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
    state: DenState,
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
    let conv_id = normalize_acp_conversation_id(Some(&conversation_id))?;
    if conv_id.starts_with("new-") {
        return Err(CustomError::ValidationError(
            "history is only available for default or saved conv-/den-conv- conversations".to_string(),
        ));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let before = query
        .before
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
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
    let (messages, has_more, next_before) = map_canonical_history_page(&canonical_rows, limit);
    let mut compaction_history = list_runtime_compaction_events(&state.sqlx_pool, &conv_id, 10)
        .await
        .unwrap_or_default();
    let latest_artifact = latest_compaction_artifact_for_conversation(
        &state.sqlx_pool,
        canonical_conversation.id,
    )
    .await
    .unwrap_or_default();
    if let (Some(latest), Some(first_event)) = (latest_artifact, compaction_history.first_mut()) {
        first_event.latest_artifact = Some(latest);
    }
    let compaction = if canonical_visible_count > 0 {
        compaction_history.first().cloned()
    } else {
        Some(crate::acp::AcpCompactionStatusResponse {
            status: "unavailable".to_string(),
            policy_version: "canonical_only".to_string(),
            trigger: None,
            created_at: None,
            source_group_start: None,
            source_group_end: None,
            diagnostic: Some(
                "Canonical ACP conversation history is not yet available for this conversation. Live provider history fallback has been disabled for pair ACP reads during migration.".to_string(),
            ),
            artifact: None,
            context_envelope: None,
            prompt_memory_diagnostic: None,
            latest_artifact: None,
        })
    };
    Ok(Json(AcpConversationHistoryResponse {
        messages,
        has_more,
        next_before,
        compaction,
        compaction_history,
    })
    .into_response())
}
