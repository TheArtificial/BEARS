//! `den`-side wiring for identity/membership/policy tools + authorization.
//!
//! The JSON shaping and the dispatcher authorization helpers now live in
//! `den_core::tools::identity`; this module provides the concrete [`BearDirectory`]
//! over the `bears`/`user` DB, wired into the dispatcher via `DenToolContext`.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use den_core::tools::identity::{BearDirectory, BearMemberRecord, BearRecord, CurrentUser};

use crate::{
    errors::{CustomError, DenError},
    core::user,
};
use den_runtime::{
    bears::{db as bears_db, BearProfile},
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
            ?;
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
            
    }

    async fn members(&self, bear_id: Uuid) -> Result<Vec<BearMemberRecord>, DenError> {
        let members = bears_db::list_members_for_bear(self.pool, bear_id)
            .await
            ?;
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

