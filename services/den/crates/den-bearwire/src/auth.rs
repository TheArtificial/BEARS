use axum::http::{header, HeaderMap};
use serde_json::Value;

use den_http::{acp_tokens, errors::CustomError};
use den_runtime::{bears::db as bears_db, DenState};

fn bearer_token(headers: &HeaderMap) -> Result<&str, CustomError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| CustomError::Authentication("missing Authorization bearer token".to_string()))?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| CustomError::Authentication("expected Authorization: Bearer <token>".to_string()))
}

pub(crate) async fn authenticate_for_bear_slug(
    state: &DenState,
    headers: &HeaderMap,
    bear_slug: &str,
) -> Result<i32, CustomError> {
    let token = bearer_token(headers)?;
    if !acp_tokens::is_acp_token(token) {
        return Err(CustomError::Authentication(
            "expected a bear-scoped BEARS ACP token".to_string(),
        ));
    }
    acp_tokens::authenticate_for_bear_slug(
        &state.sqlx_pool,
        token,
        bear_slug,
        acp_tokens::acp_chat_scope(),
    )
    .await?
    .ok_or_else(|| CustomError::Authorization("token is not valid for this Bear".to_string()))
}

pub(crate) async fn authenticated_bear(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<(i32, den_runtime::bears::Bear), CustomError> {
    let bear_slug = params
        .get("bear_slug")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CustomError::ValidationError("bear_slug is required".to_string()))?;
    let user_id = authenticate_for_bear_slug(state, headers, bear_slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, bear_slug)
        .await?
        .ok_or_else(|| CustomError::NotFound("Bear not found or token lacks access".to_string()))?;
    Ok((user_id, bear))
}
