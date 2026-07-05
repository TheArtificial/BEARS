use axum::http::{header, HeaderMap};
use serde_json::Value;

use den_http::{armature_tokens, errors::CustomError};
use den_service::{bears::db as bears_db, DenState};

fn bearer_token(headers: &HeaderMap) -> Result<&str, CustomError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            CustomError::Authentication("missing Authorization bearer token".to_string())
        })?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            CustomError::Authentication("expected Authorization: Bearer <token>".to_string())
        })
}

pub(crate) async fn authenticate_for_bear_slug(
    state: &DenState,
    headers: &HeaderMap,
    bear_slug: &str,
) -> Result<i32, CustomError> {
    let token = bearer_token(headers)?;
    let required_scope = armature_tokens::armature_chat_scope();
    if !armature_tokens::is_armature_token(token) {
        let diagnostics = armature_tokens::diagnose_for_bear_slug(
            &state.sqlx_pool,
            token,
            bear_slug,
            required_scope,
        )
        .await?;
        return Err(CustomError::Authentication(format!(
            "expected a bear-scoped BEARS Armature token; diagnostics: {}",
            diagnostics.summary()
        )));
    }
    match armature_tokens::authenticate_for_bear_slug(
        &state.sqlx_pool,
        token,
        bear_slug,
        required_scope,
    )
    .await?
    {
        Some(user_id) => Ok(user_id),
        None => {
            let diagnostics = armature_tokens::diagnose_for_bear_slug(
                &state.sqlx_pool,
                token,
                bear_slug,
                required_scope,
            )
            .await?;
            Err(CustomError::Authorization(format!(
                "token is not valid for this Bear; diagnostics: {}",
                diagnostics.summary()
            )))
        }
    }
}

pub(crate) async fn authenticated_bear(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<(i32, den_service::bears::Bear), CustomError> {
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
