//! Bear membership and session helpers shared by settings and management routes.

use axum::response::Redirect;
use uuid::Uuid;

use crate::{
    auth_backend::SessionUser,
    core::user,
    errors::CustomError,
};
use den_runtime::bears::{
    db as bears_db,
    db::role_is_bear_admin,
    Bear,
};

pub(crate) async fn email_verify_redirect(
    pool: &sqlx::PgPool,
    user_id: i32,
) -> Result<Option<Redirect>, CustomError> {
    let u = user::user_by_id(pool, user_id).await?;
    if !u.email_verified.unwrap_or(false) {
        return Ok(Some(Redirect::to("/settings/email/verify")));
    }
    Ok(None)
}

pub(crate) async fn load_bear_member(
    pool: &sqlx::PgPool,
    user_id: i32,
    slug: &str,
) -> Result<Bear, CustomError> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err(CustomError::NotFound("bear not found".to_string()));
    }
    bears_db::bear_for_user_by_slug(pool, user_id, slug)
        .await?
        .ok_or_else(|| {
            CustomError::NotFound("Bear not found or you do not have access.".to_string())
        })
}

async fn viewer_is_bear_admin(
    pool: &sqlx::PgPool,
    user_id: i32,
    bear_id: Uuid,
) -> Result<bool, CustomError> {
    let role = bears_db::membership_role_for_user(pool, user_id, bear_id).await?;
    Ok(match role {
        None => false,
        Some(inner) => role_is_bear_admin(inner.as_deref()),
    })
}

/// Edit bear settings, access, membership, delete: bear admins only.
pub(crate) async fn viewer_can_manage_bear(
    pool: &sqlx::PgPool,
    user: &SessionUser,
    bear_id: Uuid,
) -> Result<bool, CustomError> {
    viewer_is_bear_admin(pool, user.id, bear_id).await
}
