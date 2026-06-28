// ROUTES: When modifying routes in this file, update /src/web/ROUTES.md
//! End-user JSON + SSE under `/v1/*` (session cookie, same origin as Deep Chat).

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use axum_extra::extract::Query;
use axum_login::login_required;
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use uuid::Uuid;

use crate::web::bear::create_support::{
    canonical_default_model_handle, model_catalog_select_context,
};
use crate::{
    auth_backend::{AuthSession, Backend},
    core::{
        docket::{DocketService, PgDocketService},
        work_plans::{self, WorkPlanLookup},
    },
    errors::CustomError,
    observability::{
        chat_proxy_stream::{deep_chat_sse_body_for_assistant_text, BearChannelSseProxyStream},
        native_web_chat_stream::NativeWebChatUpstreamStream,
    },
    web::AppState,
    web_chat_runtime::WebChatRuntimeRequest,
};
use den_llm::ModelOption;
use den_service::{acp_sessions, archived_conversations};
use den_service::{
    bears::{
        db::{self as bears_db, role_is_bear_admin},
        BearProfile,
    },
    conversation::persistence as conversation_persistence,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/bears", get(list_my_bears))
        .route("/chat/conversations", get(chat_conversations))
        .route(
            "/chat/conversations/{conversation_id}",
            patch(chat_conversation_patch),
        )
        .route("/chat/history", get(chat_history))
        .route("/chat/model", get(chat_model_get).patch(chat_model_patch))
        .route("/chat/send", post(chat_send))
        .route_layer(login_required!(Backend, login_url = "/login"))
}

/// Membership-filtered bears for the chat UI (no provider ids exposed).
#[derive(Serialize)]
pub struct BearPublic {
    pub bear_id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    /// `user_bear.role == "admin"` for this user (bear admin, not site operator).
    pub is_bear_admin: bool,
}

async fn list_my_bears(
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Json<Vec<BearPublic>>, CustomError> {
    let user_id = auth_session
        .user
        .as_ref()
        .map(|u| u.id)
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))?;

    let rows = bears_db::list_bears_for_user(state.sqlx_pool(), user_id).await?;
    let out: Vec<BearPublic> = rows
        .into_iter()
        .map(|row| BearPublic {
            bear_id: row.bear.id,
            slug: row.bear.slug,
            name: row.bear.name,
            description: row.bear.description,
            is_bear_admin: role_is_bear_admin(row.membership_role.as_deref()),
        })
        .collect();
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
pub struct ChatHistoryQuery {
    pub bear_id: Uuid,
    /// runtime conversation: `default` (agent main conversation) or `conv-…`.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Canonical cursor: messages older than this sequence number.
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub debug: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChatConversationsQuery {
    pub bear_id: Uuid,
}

#[derive(Serialize)]
pub struct ChatConversationRow {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<String>,
}

#[derive(Serialize)]
pub struct ChatConversationsResponse {
    pub conversations: Vec<ChatConversationRow>,
}

#[derive(Debug, Deserialize)]
pub struct ChatConversationPatchBody {
    pub bear_id: Uuid,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub deleted: Option<bool>,
}

#[derive(Serialize)]
pub struct ChatConversationPatchResponse {
    pub ok: bool,
}

#[derive(Serialize)]
pub struct ChatHistoryMessage {
    pub role: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct ChatHistoryResponse {
    pub messages: Vec<ChatHistoryMessage>,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_before: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatModelQuery {
    pub bear_id: Uuid,
    #[serde(default)]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatModelPatchBody {
    pub bear_id: Uuid,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub selection_mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Serialize)]
pub struct ChatModelResponse {
    pub selection_mode: String,
    pub requested_model: Option<String>,
    pub selected_model: Option<String>,
    pub effective_model: String,
    pub source: String,
    pub model_options: Vec<ModelOption>,
}

/// `None` / empty / `default` → agent main conversation. Existing runtime conversations are `conv-...`.
/// The web UI may also send a temporary `new-...` placeholder before Den resolves the durable
/// conversation id; Codepool turns that into an SDK `createSession(agent_id)` call.
fn normalize_client_conversation_id(raw: Option<&str>) -> Result<String, CustomError> {
    let s = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    if s == "default" {
        return Ok("default".to_string());
    }
    let ok = (s.starts_with("conv-") || s.starts_with("new-"))
        && s.len() > 8
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(s.to_string())
    } else {
        Err(CustomError::ValidationError(format!(
            "invalid conversation_id (expected 'default', a runtime conv- id, or a pending new- id): {s}"
        )))
    }
}

async fn chat_conversations(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Query(q): Query<ChatConversationsQuery>,
) -> Result<Json<ChatConversationsResponse>, CustomError> {
    let user_id = auth_session
        .user
        .as_ref()
        .map(|u| u.id)
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))?;

    let allowed = bears_db::user_may_use_bear(state.sqlx_pool(), user_id, q.bear_id).await?;
    if !allowed {
        return Err(CustomError::Authorization(
            "you do not have access to this bear".to_string(),
        ));
    }

    let bear = bears_db::get_bear(state.sqlx_pool(), q.bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;

    let default_row = || ChatConversationRow {
        id: "default".to_string(),
        title: "Main chat".to_string(),
        last_message_at: None,
    };

    let archived_ids = archived_conversations::list_for_bear(state.sqlx_pool(), bear.id).await?;
    let mut conversations =
        conversation_persistence::list_conversations_for_bear(state.sqlx_pool(), bear.id, 100)
            .await?
            .into_iter()
            .filter_map(|row| {
                let id = row.external_conversation_id?;
                if id.starts_with("new-") || archived_ids.contains(&id) {
                    return None;
                }
                Some(ChatConversationRow {
                    id: id.clone(),
                    title: row
                        .current_title
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| {
                            if id == "default" {
                                "Main chat".to_string()
                            } else {
                                id.clone()
                            }
                        }),
                    last_message_at: Some(
                        row.updated_at
                            .format(&time::format_description::well_known::Rfc3339)
                            .ok()?,
                    ),
                })
            })
            .collect::<Vec<_>>();

    if !conversations.iter().any(|row| row.id == "default") {
        conversations.insert(0, default_row());
    }

    Ok(Json(ChatConversationsResponse { conversations }))
}

async fn chat_conversation_patch(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(conversation_id): Path<String>,
    Json(body): Json<ChatConversationPatchBody>,
) -> Result<Json<ChatConversationPatchResponse>, CustomError> {
    let user_id = auth_session
        .user
        .as_ref()
        .map(|u| u.id)
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))?;

    let allowed = bears_db::user_may_use_bear(state.sqlx_pool(), user_id, body.bear_id).await?;
    if !allowed {
        return Err(CustomError::Authorization(
            "you do not have access to this bear".to_string(),
        ));
    }

    let conv_id = normalize_client_conversation_id(Some(&conversation_id))?;
    if conv_id == "default" || conv_id.starts_with("new-") {
        return Err(CustomError::ValidationError(
            "only saved conversations can be renamed or archived".to_string(),
        ));
    }

    let bear = bears_db::get_bear(state.sqlx_pool(), body.bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;

    conversation_persistence::get_conversation_for_external_id(
        state.sqlx_pool(),
        bear.id,
        &conv_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("conversation not found".to_string()))?;

    let title = body
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if body.title.is_some() && title.is_none() {
        return Err(CustomError::ValidationError(
            "conversation title cannot be empty".to_string(),
        ));
    }
    if body.title.is_none() && body.archived.is_none() && body.deleted != Some(true) {
        return Err(CustomError::ValidationError(
            "no conversation update requested".to_string(),
        ));
    }

    if body.deleted == Some(true) {
        conversation_persistence::delete_conversation_for_external_id(
            state.sqlx_pool(),
            bear.id,
            &conv_id,
        )
        .await?;
        archived_conversations::set_archived(
            state.sqlx_pool(),
            bear.id,
            &conv_id,
            Some(user_id),
            "delete",
            false,
        )
        .await?;
        return Ok(Json(ChatConversationPatchResponse { ok: true }));
    }

    if let Some(title) = title {
        let title = title.chars().take(120).collect::<String>();
        let _ = conversation_persistence::set_conversation_title(
            state.sqlx_pool(),
            bear.id,
            &conv_id,
            &title,
        )
        .await?;
        let _ = acp_sessions::set_title_for_bear_conversation(
            state.sqlx_pool(),
            bear.id,
            &conv_id,
            &title,
        )
        .await?;
    }

    if let Some(archived) = body.archived {
        archived_conversations::set_archived(
            state.sqlx_pool(),
            bear.id,
            &conv_id,
            Some(user_id),
            "web",
            archived,
        )
        .await?;
    }

    Ok(Json(ChatConversationPatchResponse { ok: true }))
}

async fn chat_history(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Query(q): Query<ChatHistoryQuery>,
) -> Result<Json<ChatHistoryResponse>, CustomError> {
    let user_id = auth_session
        .user
        .as_ref()
        .map(|u| u.id)
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))?;

    let allowed = bears_db::user_may_use_bear(state.sqlx_pool(), user_id, q.bear_id).await?;
    if !allowed {
        return Err(CustomError::Authorization(
            "you do not have access to this bear".to_string(),
        ));
    }

    let bear = bears_db::get_bear(state.sqlx_pool(), q.bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;

    let empty = || {
        Json(ChatHistoryResponse {
            messages: vec![],
            has_more: false,
            next_before: None,
        })
    };

    let limit = i64::from(q.limit.unwrap_or(50).clamp(1, 100));
    let before_sequence_no = q
        .before
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::parse::<i64>)
        .transpose()
        .map_err(|_| {
            CustomError::ValidationError("before must be a canonical sequence number".to_string())
        })?;

    let conv_id = normalize_client_conversation_id(q.conversation_id.as_deref())?;

    let Some(conversation) = conversation_persistence::get_conversation_for_external_id(
        state.sqlx_pool(),
        bear.id,
        &conv_id,
    )
    .await?
    else {
        return Ok(empty());
    };

    let rows = conversation_persistence::list_messages_page(
        state.sqlx_pool(),
        conversation.id,
        before_sequence_no,
        limit,
    )
    .await?;
    let (messages, has_more, next_before) = map_persisted_history_page(&rows, limit as usize);
    Ok(Json(ChatHistoryResponse {
        messages,
        has_more,
        next_before,
    }))
}

/// Deep Chat history expects `ai`; Postgres stores `assistant`.
fn client_chat_history_role(storage_role: &str) -> String {
    match storage_role {
        "assistant" => "ai".to_string(),
        other => other.to_string(),
    }
}

fn map_persisted_history_page(
    rows: &[conversation_persistence::PersistedConversationMessage],
    page_limit: usize,
) -> (Vec<ChatHistoryMessage>, bool, Option<String>) {
    let visible_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.is_transcript_visible())
        .collect();

    let mut coalesced_desc: Vec<(i64, ChatHistoryMessage)> = Vec::new();
    for row in visible_rows {
        let storage_role = row.role.clone().unwrap_or_else(|| "assistant".to_string());
        if let Some((_, last)) = coalesced_desc.last_mut() {
            if last.role == client_chat_history_role(&storage_role)
                && storage_role == "assistant"
                && matches!(
                    row.storage_message_type(),
                    Ok(den_service::conversation::message_types::ConversationMessageType::Assistant)
                )
            {
                last.text.push_str(&row.content_text);
                last.text =
                    crate::observability::chat_proxy_stream::strip_ephemeral_status_suffixes(
                        &last.text,
                    );
                continue;
            }
        }
        let text = crate::observability::chat_proxy_stream::strip_ephemeral_status_suffixes(
            &row.content_text,
        );
        if text.is_empty() {
            continue;
        }
        coalesced_desc.push((
            row.sequence_no,
            ChatHistoryMessage {
                role: client_chat_history_role(&storage_role),
                text,
            },
        ));
    }

    let has_more = coalesced_desc.len() >= page_limit;
    let page = coalesced_desc
        .into_iter()
        .take(page_limit)
        .collect::<Vec<_>>();
    let next_before = page.last().map(|(sequence_no, _)| sequence_no.to_string());
    let messages = page
        .into_iter()
        .rev()
        .map(|(_, message)| message)
        .collect::<Vec<_>>();
    (messages, has_more, next_before)
}

#[derive(Debug, Deserialize)]
pub struct ChatSendRequest {
    pub bear_id: Uuid,
    pub message: String,
    /// Reserved for runtime conversation / OTID pass-through (optional).
    #[serde(default)]
    pub conversation_id: Option<String>,
}

fn chat_send_api_status_message(err: &CustomError) -> (StatusCode, String) {
    match err {
        CustomError::Anyhow(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        CustomError::System(s) => (StatusCode::UNPROCESSABLE_ENTITY, s.clone()),
        CustomError::Database(s) => (StatusCode::UNPROCESSABLE_ENTITY, s.clone()),
        CustomError::DatabaseUnavailable(s) => (StatusCode::SERVICE_UNAVAILABLE, s.clone()),
        CustomError::Session(s) => (StatusCode::INTERNAL_SERVER_ERROR, s.clone()),
        CustomError::Authentication(s) => (StatusCode::UNAUTHORIZED, s.clone()),
        CustomError::Authorization(s) => (StatusCode::FORBIDDEN, s.clone()),
        CustomError::Render(s) => (StatusCode::INTERNAL_SERVER_ERROR, s.clone()),
        CustomError::Parsing(s) => (StatusCode::UNPROCESSABLE_ENTITY, s.clone()),
        CustomError::Email(s) => (StatusCode::FAILED_DEPENDENCY, s.clone()),
        CustomError::NotFound(s) => (StatusCode::NOT_FOUND, s.clone()),
        CustomError::ValidationError(s) => (StatusCode::BAD_REQUEST, s.clone()),
    }
}

fn chat_send_error_response(err: CustomError, request_id: Uuid) -> Response {
    tracing::error!(%request_id, error = %err, "chat_send rejected");
    let (status, message) = chat_send_api_status_message(&err);
    let body = serde_json::json!({
        "error": message,
        "request_id": request_id,
    });
    let request_id_header = HeaderValue::from_str(&request_id.to_string())
        .unwrap_or_else(|_| HeaderValue::from_static("invalid"));
    match Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(HeaderName::from_static("x-request-id"), request_id_header)
        .body(Body::from(body.to_string()))
    {
        Ok(r) => r,
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("response build: {e}"),
        )
            .into_response(),
    }
}

async fn web_chat_workboard_prompt_context(
    pool: &sqlx::PgPool,
    bear_id: Uuid,
    user_id: i32,
    conversation_id: &str,
    _session_id: &str,
) -> Result<String, CustomError> {
    let plan = PgDocketService::from_pool(pool)
        .get_visible_work_plan(
            bear_id,
            BearProfile::Chat,
            user_id,
            WorkPlanLookup {
                plan_id: None,
                source_conversation_id: Some(conversation_id.to_string()),
                source_acp_session_id: None,
            },
        )
        .await?;
    let plans = plan.into_iter().collect::<Vec<_>>();
    Ok(work_plans::render_workboard_prompt_context(&plans))
}

fn chat_model_available(options: &[ModelOption], raw: &str) -> bool {
    let requested = raw.trim();
    if requested.is_empty() {
        return false;
    }
    let requested_resolved = den_llm::model_registry::resolve_model_handle(requested);
    options.iter().any(|model| {
        if model.handle == requested {
            return true;
        }
        let Some(resolved) = requested_resolved else {
            return false;
        };
        resolved == model.handle
            || den_llm::model_registry::resolve_model_handle(&model.handle) == Some(resolved)
    })
}

async fn chat_model_response_for(
    state: &AppState,
    user_id: i32,
    bear_id: Uuid,
    conversation_id: Option<&str>,
) -> Result<ChatModelResponse, CustomError> {
    let allowed = bears_db::user_may_use_bear(state.sqlx_pool(), user_id, bear_id).await?;
    if !allowed {
        return Err(CustomError::Authorization(
            "you do not have access to this bear".to_string(),
        ));
    }
    let bear = bears_db::get_bear(state.sqlx_pool(), bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    let conv_id = normalize_client_conversation_id(conversation_id)?;
    let (configured, model_options, fetch_error) = model_catalog_select_context(state).await;
    if !configured || model_options.is_empty() {
        return Err(CustomError::System(fetch_error.unwrap_or_else(|| {
            "No Den model selection options are configured.".to_string()
        })));
    }

    let base_model = bears_db::resolve_model_for_profile(
        state.sqlx_pool(),
        &bear,
        BearProfile::Chat,
        state.config.default_llm_model.as_str(),
    )
    .await?;

    if conv_id.starts_with("new-") {
        return Ok(ChatModelResponse {
            selection_mode: "auto".to_string(),
            requested_model: None,
            selected_model: None,
            effective_model: base_model,
            source: "stance_or_bear_default".to_string(),
            model_options,
        });
    }

    let conversation = conversation_persistence::ensure_conversation_for_external_id(
        state.sqlx_pool(),
        bear.id,
        Some(user_id),
        &conv_id,
        None,
        None,
    )
    .await?;
    let state_row =
        conversation_persistence::get_conversation_model_state(state.sqlx_pool(), conversation.id)
            .await?;
    let effective = conversation_persistence::resolve_conversation_selected_model(
        state.sqlx_pool(),
        conversation.id,
    )
    .await?
    .unwrap_or(base_model);
    Ok(ChatModelResponse {
        selection_mode: state_row
            .as_ref()
            .map(|row| row.selection_mode.clone())
            .unwrap_or_else(|| "auto".to_string()),
        requested_model: state_row
            .as_ref()
            .and_then(|row| row.requested_model.clone()),
        selected_model: state_row
            .as_ref()
            .and_then(|row| row.selected_model.clone()),
        effective_model: effective,
        source: if state_row.as_ref().map(|row| row.selection_mode.as_str()) == Some("explicit") {
            "conversation_explicit".to_string()
        } else {
            "stance_or_bear_default".to_string()
        },
        model_options,
    })
}

async fn chat_model_get(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Query(q): Query<ChatModelQuery>,
) -> Result<Json<ChatModelResponse>, CustomError> {
    let user_id = auth_session
        .user
        .as_ref()
        .map(|u| u.id)
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))?;
    Ok(Json(
        chat_model_response_for(&state, user_id, q.bear_id, q.conversation_id.as_deref()).await?,
    ))
}

async fn chat_model_patch(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Json(body): Json<ChatModelPatchBody>,
) -> Result<Json<ChatModelResponse>, CustomError> {
    let user_id = auth_session
        .user
        .as_ref()
        .map(|u| u.id)
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))?;
    let allowed = bears_db::user_may_use_bear(state.sqlx_pool(), user_id, body.bear_id).await?;
    if !allowed {
        return Err(CustomError::Authorization(
            "you do not have access to this bear".to_string(),
        ));
    }
    let bear = bears_db::get_bear(state.sqlx_pool(), body.bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;
    let conv_id = normalize_client_conversation_id(body.conversation_id.as_deref())?;
    if conv_id.starts_with("new-") {
        return Err(CustomError::ValidationError(
            "choose a model after the conversation is created".to_string(),
        ));
    }
    let (configured, model_options, fetch_error) = model_catalog_select_context(&state).await;
    if !configured || model_options.is_empty() {
        return Err(CustomError::System(fetch_error.unwrap_or_else(|| {
            "No Den model selection options are configured.".to_string()
        })));
    }
    let mode = body.selection_mode.as_deref().unwrap_or("auto").trim();
    let (selection_mode, requested_model, selected_model, reason) = if mode == "explicit" {
        let raw = body.model.as_deref().unwrap_or("").trim();
        if raw.is_empty() || !chat_model_available(&model_options, raw) {
            return Err(CustomError::ValidationError(
                "Pick a configured Den model selection option.".to_string(),
            ));
        }
        let canonical = canonical_default_model_handle(raw).unwrap_or_else(|| raw.to_string());
        (
            "explicit",
            Some(canonical.clone()),
            Some(canonical),
            "human_selected",
        )
    } else {
        ("auto", None, None, "inherit_stance_or_bear_default")
    };
    let conversation = conversation_persistence::ensure_conversation_for_external_id(
        state.sqlx_pool(),
        bear.id,
        Some(user_id),
        &conv_id,
        None,
        None,
    )
    .await?;
    conversation_persistence::set_conversation_model_state(
        state.sqlx_pool(),
        conversation.id,
        selection_mode,
        requested_model.as_deref(),
        selected_model.as_deref(),
        Some(reason),
    )
    .await?;
    Ok(Json(
        chat_model_response_for(&state, user_id, body.bear_id, Some(&conv_id)).await?,
    ))
}

async fn chat_send(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Json(body): Json<ChatSendRequest>,
) -> impl IntoResponse {
    let request_id = Uuid::new_v4();
    let result = async { chat_send_inner(state, auth_session, body, request_id).await }
        .instrument(tracing::info_span!("chat_send", request_id = %request_id))
        .await;
    match result {
        Ok(r) => r.into_response(),
        Err(e) => chat_send_error_response(e, request_id),
    }
}

fn parse_set_conversation_title_request(message: &str) -> Option<String> {
    let trimmed = message.trim();
    let lower = trimmed.to_ascii_lowercase();
    for prefix in [
        "set conversation title to ",
        "rename conversation to ",
        "rename this conversation to ",
        "set this conversation title to ",
    ] {
        if lower.starts_with(prefix) {
            return Some(
                trimmed[prefix.len()..]
                    .trim()
                    .trim_matches(['\"', '\''])
                    .to_string(),
            )
            .filter(|title| !title.is_empty());
        }
    }
    None
}

struct ConversationTitleRequest<'a> {
    bear: &'a den_service::bears::Bear,
    conv_id: &'a str,
    message: &'a str,
    request_id: Uuid,
}

async fn maybe_handle_direct_set_conversation_title(
    state: &AppState,
    request: ConversationTitleRequest<'_>,
) -> Result<Option<Response>, CustomError> {
    let ConversationTitleRequest {
        bear,
        conv_id,
        message,
        request_id,
    } = request;
    let Some(title) = parse_set_conversation_title_request(message) else {
        return Ok(None);
    };
    let title = title.chars().take(120).collect::<String>();
    conversation_persistence::set_conversation_title(state.sqlx_pool(), bear.id, conv_id, &title)
        .await?;
    let _ =
        acp_sessions::set_title_for_bear_conversation(state.sqlx_pool(), bear.id, conv_id, &title)
            .await?;
    let text = "Conversation title updated.";
    let body = deep_chat_sse_body_for_assistant_text(text);
    let request_id_header = HeaderValue::from_str(&request_id.to_string())
        .map_err(|_| CustomError::System("invalid request id for response header".to_string()))?;
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header(HeaderName::from_static("x-request-id"), request_id_header)
        .body(Body::from(body))
        .map_err(|err| CustomError::System(format!("response build: {err}")))?;
    Ok(Some(response))
}

fn direct_chat_sse_response(text: &str, request_id: Uuid) -> Result<Response, CustomError> {
    let body = deep_chat_sse_body_for_assistant_text(text);
    let request_id_header = HeaderValue::from_str(&request_id.to_string())
        .map_err(|_| CustomError::System("invalid request id for response header".to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header(HeaderName::from_static("x-request-id"), request_id_header)
        .body(Body::from(body))
        .map_err(|err| CustomError::System(format!("response build: {err}")))
}

fn chat_turn_is_capabilities_meta_query(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    const PHRASES: &[&str] = &[
        "list capabilities",
        "list your capabilities",
        "list tools",
        "list your tools",
        "what tools",
        "what capabilities",
        "which tools",
        "which capabilities",
    ];
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

async fn maybe_handle_direct_capabilities_list(
    pool: &sqlx::PgPool,
    canonical_conversation_id: Uuid,
    message: &str,
    request_id: Uuid,
) -> Result<Option<Response>, CustomError> {
    if !chat_turn_is_capabilities_meta_query(message.trim()) {
        return Ok(None);
    }
    let text = den_core::tools::descriptor::render_profile_tool_surface_blurb(BearProfile::Chat);
    conversation_persistence::append_message(
        pool,
        canonical_conversation_id,
        &den_service::conversation::message_types::ConversationMessageWrite::assistant_turn(
            text.clone(),
            serde_json::json!({
                "type": "assistant_output",
                "text": text,
                "request_id": request_id.to_string(),
                "source": "direct_capabilities_list",
            }),
        ),
    )
    .await?;
    tracing::info!(
        %request_id,
        conversation_id = %canonical_conversation_id,
        "web chat capabilities list answered without LLM round-trip"
    );
    Ok(Some(direct_chat_sse_response(&text, request_id)?))
}

async fn resolve_chat_profile_binding_id(
    pool: &sqlx::PgPool,
    bear_id: Uuid,
    native_runtime: bool,
) -> Result<String, CustomError> {
    bears_db::profile_binding_id(pool, bear_id, BearProfile::Chat)
        .await?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CustomError::System(if native_runtime {
                "This bear has no chat profile runtime binding. Ask an operator to provision missing profiles in Admin → Bears.".to_string()
            } else {
                "This bear is not provisioned yet (missing chat profile runtime)."
                    .to_string()
            })
        })
}

fn chat_sse_response(
    stream: BearChannelSseProxyStream,
    request_id: Uuid,
) -> Result<Response, CustomError> {
    let request_id_header = HeaderValue::from_str(&request_id.to_string())
        .map_err(|_| CustomError::System("invalid request id for response header".to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header(HeaderName::from_static("x-request-id"), request_id_header)
        .body(Body::from_stream(stream))
        .map_err(|e| CustomError::System(format!("response build: {e}")))
}

async fn chat_send_native_inner(
    state: AppState,
    body: ChatSendRequest,
    request_id: Uuid,
    user_id: i32,
    username: &str,
    bear: den_service::bears::Bear,
    chat_binding_id: &str,
    conv_id: String,
) -> Result<Response, CustomError> {
    if state.config.llm_api_url.trim().is_empty() {
        return Err(CustomError::System(
            "Chat is unavailable: LLM_API_URL is not set (required when AGENT_RUNTIME=native)."
                .to_string(),
        ));
    }

    let membership_role =
        bears_db::membership_role_for_user(state.sqlx_pool(), user_id, body.bear_id)
            .await?
            .flatten();
    let session_id = format!("den-web:{}:{}", body.bear_id, conv_id);
    if let Some(response) = maybe_handle_direct_set_conversation_title(
        &state,
        ConversationTitleRequest {
            bear: &bear,
            conv_id: &conv_id,
            message: body.message.trim(),
            request_id,
        },
    )
    .await?
    {
        return Ok(response);
    }

    let workboard_context = web_chat_workboard_prompt_context(
        state.sqlx_pool(),
        bear.id,
        user_id,
        &conv_id,
        &session_id,
    )
    .await?;
    let upstream_message = format!("{}{}", body.message.trim(), workboard_context);

    let canonical_conversation = conversation_persistence::ensure_conversation_for_external_id(
        state.sqlx_pool(),
        bear.id,
        Some(user_id),
        &conv_id,
        None,
        None,
    )
    .await?;
    conversation_persistence::append_message(
        state.sqlx_pool(),
        canonical_conversation.id,
        &den_service::conversation::message_types::ConversationMessageWrite::user_turn(
            body.message.trim(),
            serde_json::json!({
                "type": "user_input",
                "text": body.message.trim(),
                "request_id": request_id.to_string(),
            }),
            Some(format!("web-chat-user-input:{request_id}")),
        ),
    )
    .await?;

    if let Some(response) = maybe_handle_direct_capabilities_list(
        state.sqlx_pool(),
        canonical_conversation.id,
        body.message.trim(),
        request_id,
    )
    .await?
    {
        return Ok(response);
    }

    crate::observability::metrics::chat_send_runtime_native();

    let runtime_stream = state
        .web_chat_runtime
        .stream_chat(
            &state,
            WebChatRuntimeRequest {
                bear_id: bear.id,
                bear_slug: bear.slug.clone(),
                chat_binding_id: chat_binding_id.to_string(),
                user_id,
                username: Some(username.to_string()),
                membership_role: membership_role.clone(),
                conversation_id: conv_id.clone(),
                session_id: session_id.clone(),
                prompt: upstream_message,
                request_id,
            },
        )
        .await?;

    crate::observability::metrics::chat_send_started();

    let upstream = NativeWebChatUpstreamStream::new(runtime_stream, request_id);
    let stream = BearChannelSseProxyStream::new(
        upstream,
        request_id,
        user_id,
        body.bear_id,
        conv_id,
        state.sqlx_pool().clone(),
    );
    chat_sse_response(stream, request_id)
}

async fn chat_send_inner(
    state: AppState,
    auth_session: AuthSession,
    body: ChatSendRequest,
    request_id: Uuid,
) -> Result<Response, CustomError> {
    let session_user = auth_session
        .user
        .as_ref()
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))?;
    let user_id = session_user.id;
    let username = session_user.username.clone();

    if body.message.trim().is_empty() {
        return Err(CustomError::ValidationError(
            "message must not be empty".to_string(),
        ));
    }

    let allowed = bears_db::user_may_use_bear(state.sqlx_pool(), user_id, body.bear_id).await?;
    if !allowed {
        return Err(CustomError::Authorization(
            "you do not have access to this bear".to_string(),
        ));
    }

    let bear = bears_db::get_bear(state.sqlx_pool(), body.bear_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("bear not found".to_string()))?;

    let chat_binding_id = resolve_chat_profile_binding_id(state.sqlx_pool(), bear.id, true).await?;
    let conv_id = normalize_client_conversation_id(body.conversation_id.as_deref())?;

    chat_send_native_inner(
        state,
        body,
        request_id,
        user_id,
        username.as_str(),
        bear,
        &chat_binding_id,
        conv_id,
    )
    .await
}

#[cfg(test)]
mod chat_history_map_tests {
    use super::*;
    use den_service::conversation::persistence::PersistedConversationMessage;

    fn persisted_row(
        sequence_no: i64,
        role: &str,
        message_type: &str,
        text: &str,
    ) -> PersistedConversationMessage {
        PersistedConversationMessage {
            sequence_no,
            message_type: message_type.to_string(),
            role: Some(role.to_string()),
            visibility: "default".to_string(),
            content_text: text.to_string(),
            provider_message_id: None,
            created_at: time::OffsetDateTime::now_utc(),
        }
    }

    #[test]
    fn map_persisted_page_strips_trailing_ephemeral_status_suffix() {
        let rows = vec![
            persisted_row(2, "assistant", "assistant", "HelloThinking…"),
            persisted_row(1, "user", "user", "Hello"),
        ];
        let (msgs, has_more, _) = map_persisted_history_page(&rows, 10);
        assert!(!has_more);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].text, "Hello");
    }

    #[test]
    fn map_persisted_page_omits_ephemeral_only_assistant_rows() {
        let rows = vec![
            persisted_row(2, "assistant", "assistant", "Thinking…"),
            persisted_row(1, "user", "user", "Hello"),
        ];
        let (msgs, _, _) = map_persisted_history_page(&rows, 10);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn map_persisted_page_emits_ai_role_for_assistant_rows() {
        // `list_messages_page` returns rows newest-first (sequence DESC).
        let rows = vec![
            persisted_row(2, "assistant", "assistant", "Hi there"),
            persisted_row(1, "user", "user", "Hello"),
        ];
        let (msgs, has_more, _) = map_persisted_history_page(&rows, 10);
        assert!(!has_more);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].text, "Hello");
        assert_eq!(msgs[1].role, "ai");
        assert_eq!(msgs[1].text, "Hi there");
    }
}

#[cfg(test)]
mod conversation_id_tests {
    use super::normalize_client_conversation_id;

    #[test]
    fn normalizes_default_aliases() {
        assert_eq!(normalize_client_conversation_id(None).unwrap(), "default");
        assert_eq!(
            normalize_client_conversation_id(Some("")).unwrap(),
            "default"
        );
        assert_eq!(
            normalize_client_conversation_id(Some("default")).unwrap(),
            "default"
        );
    }

    #[test]
    fn accepts_conv_prefix_ids() {
        assert_eq!(
            normalize_client_conversation_id(Some("conv-abc12345")).unwrap(),
            "conv-abc12345"
        );
    }

    #[test]
    fn accepts_pending_new_prefix_ids() {
        assert_eq!(
            normalize_client_conversation_id(Some("new-abc12345")).unwrap(),
            "new-abc12345"
        );
    }

    #[test]
    fn rejects_garbage_ids() {
        assert!(normalize_client_conversation_id(Some("../../../etc/passwd")).is_err());
        assert!(normalize_client_conversation_id(Some("conv-x")).is_err());
    }
}
