//! `den`-side wiring for identity/membership/policy tools + authorization.
//!
//! The JSON shaping and the dispatcher authorization helpers now live in
//! `den_tools::identity`; this module provides the concrete [`BearDirectory`]
//! over the `bears`/`user` DB and thin wrappers used by the dispatcher.

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use den_tools::identity::{BearDirectory, BearMemberRecord, BearRecord, CurrentUser};

use crate::{
    core::{
        bears::{db as bears_db, BearProfile},
        tools::session::DenToolInvocationContext,
        user,
    },
    errors::{CustomError, DenError},
};

fn format_rfc3339(value: time::OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| value.to_string())
}

/// Concrete [`BearDirectory`] over the Postgres pool.
pub(crate) struct DenBearDirectory<'a> {
    pub(crate) pool: &'a PgPool,
}

#[async_trait]
impl BearDirectory for DenBearDirectory<'_> {
    async fn user_may_use_bear(&self, user_id: i32, bear_id: Uuid) -> Result<bool, DenError> {
        bears_db::user_may_use_bear(self.pool, user_id, bear_id)
            .await
            .map_err(CustomError::into_den)
    }

    async fn registered_profile(
        &self,
        bear_id: Uuid,
        binding_id: &str,
    ) -> Result<Option<BearProfile>, DenError> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"
            SELECT profile
            FROM bear_profile_bindings
            WHERE bear_id = $1
              AND binding_id = $2
            "#,
        )
        .bind(bear_id)
        .bind(binding_id)
        .fetch_optional(self.pool)
        .await
        .map_err(|err| CustomError::from(err).into_den())?;
        match row {
            None => Ok(None),
            Some((profile,)) => profile
                .parse::<BearProfile>()
                .map(Some)
                .map_err(DenError::System),
        }
    }

    async fn bear_self(&self, bear_id: Uuid) -> Result<Option<BearRecord>, DenError> {
        let bear = bears_db::get_bear(self.pool, bear_id)
            .await
            .map_err(CustomError::into_den)?;
        Ok(bear.map(|bear| BearRecord {
            id: bear.id,
            slug: bear.slug,
            name: bear.name,
            description: Some(bear.description),
            default_model: bear.default_model,
            letta_agent_type: bear.letta_agent_type,
            created_at: format_rfc3339(bear.created_at),
            updated_at: format_rfc3339(bear.updated_at),
        }))
    }

    async fn member_count(&self, bear_id: Uuid) -> Result<i64, DenError> {
        bears_db::count_bear_members(self.pool, bear_id)
            .await
            .map_err(CustomError::into_den)
    }

    async fn members(&self, bear_id: Uuid) -> Result<Vec<BearMemberRecord>, DenError> {
        let members = bears_db::list_members_for_bear(self.pool, bear_id)
            .await
            .map_err(CustomError::into_den)?;
        Ok(members
            .into_iter()
            .map(|member| BearMemberRecord {
                user_id: member.user_id,
                username: member.username,
                display_name: Some(member.display_name),
                role: member.role,
            })
            .collect())
    }

    async fn current_user(&self, user_id: i32) -> Result<CurrentUser, DenError> {
        let current = user::user_by_id(self.pool, user_id)
            .await
            .map_err(CustomError::into_den)?;
        Ok(CurrentUser {
            id: current.id,
            username: current.username,
            display_name: Some(current.display_name),
            email_verified: current.email_verified.unwrap_or(false),
            created_at: format_rfc3339(current.created.assume_utc()),
        })
    }
}

pub(crate) fn directory(pool: &PgPool) -> DenBearDirectory<'_> {
    DenBearDirectory { pool }
}

pub(crate) async fn get_bear_self(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    den_tools::identity::get_bear_self(&directory(pool), context)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn get_current_user(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    den_tools::identity::get_current_user(&directory(pool), context)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn list_bear_members(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    den_tools::identity::list_bear_members(&directory(pool), context)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn policy_self(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    den_tools::identity::policy_self(&directory(pool), context)
        .await
        .map_err(CustomError::from)
}

pub(crate) async fn list_capabilities_self(
    pool: &PgPool,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    let role = den_tools::identity::context_role(&directory(pool), context)
        .await
        .map_err(CustomError::from)?;
    Ok(den_tools::identity::list_capabilities_self(context, role))
}
