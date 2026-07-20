use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::service::DenState;
use den_http::errors::CustomError;
use den_http::user;
use den_oauth::{
    auth::extract_bearer_token_oauth,
    oauth::{error::OAuthError, jwt::create_jwt_manager},
};

#[derive(Serialize, ToSchema)]
pub struct ProfileResponse {
    /// User's unique identifier
    #[schema(example = 123)]
    pub id: i32,
    /// User's display name
    #[schema(example = "John Doe")]
    pub name: String,
    /// User's email address
    #[schema(example = "john@example.com")]
    pub email: String,
    /// Relative URL to user's profile page
    #[schema(example = "/johndoe")]
    pub profile_url: String,
    /// Whether the user's email is verified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
    /// User's theme preference (system, dark, light)
    #[schema(example = "dark")]
    pub theme: String,
    /// Day of week when user's week starts (1=Monday)
    #[schema(example = 1)]
    pub week_start_day: i32,
}

pub fn router() -> Router<DenState> {
    Router::new().route("/me", get(get_profile))
}

#[utoipa::path(
    get,
    path = "/v1.0/me",
    responses(
        (status = 200, description = "User profile retrieved successfully", body = ProfileResponse),
        (status = 403, description = "Authentication required")
    ),
    tag = "Profile"
)]
/// Get authenticated user's profile
pub async fn get_profile(
    State(state): State<DenState>,
    headers: HeaderMap,
) -> Result<Response, CustomError> {
    // Extract Bearer token from Authorization header
    let access_token = match extract_bearer_token_oauth(&headers) {
        Ok(token) => token,
        Err(oauth_error) => return Ok(bearer_error_response(oauth_error)),
    };

    // Validate JWT access token
    let jwt_manager = create_jwt_manager();
    let jwt_claims = match jwt_manager.validate_access_token(&access_token) {
        Ok(claims) => claims,
        Err(oauth_error) => return Ok(bearer_error_response(oauth_error)),
    };

    // Get user ID from JWT claims
    let user_id = match jwt_claims.user_id() {
        Ok(id) => id,
        Err(oauth_error) => return Ok(bearer_error_response(oauth_error)),
    };

    // Get user information from database
    let user = user::user_by_id(&state.sqlx_pool, user_id).await?;

    let profile = ProfileResponse {
        id: user.id,
        name: user.display_name.clone(),
        email: user.email.clone(),
        profile_url: format!("/{}", user.username),
        email_verified: user.email_verified,
        theme: user.theme.clone(),
        week_start_day: user.week_start_day,
    };

    Ok(Json(profile).into_response())
}

/// Create Bearer token error response
fn bearer_error_response(error: OAuthError) -> Response {
    let status_code = error.status_code();
    let error_code = error.error_code();
    let error_description = error.error_description();

    let www_authenticate = if status_code == StatusCode::UNAUTHORIZED {
        format!("Bearer error=\"{error_code}\", error_description=\"{error_description}\"")
    } else {
        "Bearer".to_string()
    };

    let error_response = serde_json::json!({
        "error": error_code,
        "error_description": error_description
    });

    let mut response = (status_code, Json(error_response)).into_response();
    response.headers_mut().insert(
        "WWW-Authenticate",
        HeaderValue::from_str(&www_authenticate)
            .unwrap_or_else(|_| HeaderValue::from_static("Bearer")),
    );

    response
}
