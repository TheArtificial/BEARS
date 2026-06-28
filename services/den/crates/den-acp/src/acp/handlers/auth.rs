use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use crate::{
    acp::responses::acp_error_response,
    service::DenState,
    core::armature_tokens,
};
use den_http::errors::CustomError;
use den_oauth::{auth, oauth::OAuthScope};

pub(in crate::acp) async fn auth_check(
    State(state): State<DenState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Response {
    let request_id = Uuid::new_v4();
    match authenticate_acp_code_token(&state, &headers, &slug).await {
        Ok(user_id) => Json(serde_json::json!({
            "ok": true,
            "user_id": user_id,
            "scopes": {
                "armature:chat": true
            }
        }))
        .into_response(),
        Err(err) => acp_error_response(err, request_id),
    }
}

pub(in crate::acp) async fn authenticate_acp_code_token(
    state: &DenState,
    headers: &HeaderMap,
    slug: &str,
) -> Result<i32, CustomError> {
    let token = auth::extract_bearer_token(headers)
        .map_err(|err| CustomError::Authentication(err.message))?;
    Ok(authenticate_acp_code_token_with_auth(state, &token, slug)
        .await?
        .user_id)
}

pub(in crate::acp) async fn authenticate_acp_code_token_with_auth(
    state: &DenState,
    token: &str,
    slug: &str,
) -> Result<armature_tokens::ArmatureTokenAuth, CustomError> {
    let required_scope = OAuthScope::ArmatureChat.as_str();
    if !armature_tokens::is_armature_token(token) {
        let diagnostics = armature_tokens::diagnose_for_bear_slug(
            &state.sqlx_pool,
            token,
            slug,
            required_scope,
        )
        .await?;
        return Err(CustomError::Authentication(format!(
            "expected a bear-scoped BEARS Armature token; diagnostics: {}",
            diagnostics.summary()
        )));
    }
    let auth = match armature_tokens::authenticate_for_bear_slug_with_scopes(&state.sqlx_pool, token, slug)
        .await?
    {
        Some(auth) => auth,
        None => {
            let diagnostics = armature_tokens::diagnose_for_bear_slug(
                &state.sqlx_pool,
                token,
                slug,
                required_scope,
            )
            .await?;
            return Err(CustomError::Authentication(format!(
                "invalid, expired, revoked, or unauthorized Armature token; diagnostics: {}",
                diagnostics.summary()
            )));
        }
    };
    if !armature_tokens::scopes_contains(&auth.scopes, required_scope) {
        let diagnostics = armature_tokens::diagnose_for_bear_slug(
            &state.sqlx_pool,
            token,
            slug,
            required_scope,
        )
        .await?;
        return Err(CustomError::Authentication(format!(
            "Armature token is missing required armature:chat scope; diagnostics: {}",
            diagnostics.summary()
        )));
    }
    Ok(auth)
}
