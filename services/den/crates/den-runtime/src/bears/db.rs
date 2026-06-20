//! SQL for bears and `user_bear` (runtime `query_as` — see `model.rs`).

use sqlx::{types::Json, FromRow, PgPool};
use uuid::Uuid;

use den_core::DenError;

use super::model::{
    Bear, BearProfile, BearProfileBinding, BearSkillManifestEntry, BearSkillProposal,
    BearWithMembership,
};

pub struct BearParams<'a> {
    pub slug: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub system_prompt: &'a str,
    pub default_model: Option<&'a str>,
    pub tools_enabled: Option<Json<serde_json::Value>>,
    /// Deprecated legacy provider field; active Den-native provisioning ignores it.
    pub letta_agent_type: Option<&'a str>,
    /// Deprecated legacy provider field; active Den-native provisioning ignores it.
    pub letta_tool_ids: Json<Vec<String>>,
    pub context_profile: Option<Json<serde_json::Value>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct BearProfileModelSetting {
    pub bear_id: Uuid,
    pub profile: String,
    pub model: Option<String>,
}

pub async fn list_bears(pool: &PgPool) -> Result<Vec<Bear>, DenError> {
    sqlx::query_as::<_, Bear>(
        r"
        SELECT id, slug, name, description, default_model, tools_enabled,
               letta_agent_type, letta_tool_ids, runtime_plan, context_profile,
               memfs_repo_path, provisioning_version, system_prompt, birthday, created_at, updated_at
        FROM bears
        ORDER BY slug
        ",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn get_bear(pool: &PgPool, id: Uuid) -> Result<Option<Bear>, DenError> {
    sqlx::query_as::<_, Bear>(
        r"
        SELECT id, slug, name, description, default_model, tools_enabled,
               letta_agent_type, letta_tool_ids, runtime_plan, context_profile,
               memfs_repo_path, provisioning_version, system_prompt, birthday, created_at, updated_at
        FROM bears
        WHERE id = $1
        ",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn bear_slug_exists(pool: &PgPool, slug: &str) -> Result<bool, DenError> {
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM bears WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await?;
    Ok(n.0 > 0)
}

pub async fn bear_slug_exists_excluding(
    pool: &PgPool,
    slug: &str,
    exclude_id: Uuid,
) -> Result<bool, DenError> {
    let n: (i64,) =
        sqlx::query_as("SELECT COUNT(*)::bigint FROM bears WHERE slug = $1 AND id <> $2")
            .bind(slug)
            .bind(exclude_id)
            .fetch_one(pool)
            .await?;
    Ok(n.0 > 0)
}

pub async fn update_bear(pool: &PgPool, id: Uuid, params: BearParams<'_>) -> Result<(), DenError> {
    let r = sqlx::query(
        r"
        UPDATE bears
        SET slug = $1,
            name = $2,
            description = $3,
            system_prompt = $4,
            default_model = $5,
            tools_enabled = $6,
            letta_agent_type = $7,
            letta_tool_ids = $8,
            updated_at = NOW()
        WHERE id = $9
        ",
    )
    .bind(params.slug)
    .bind(params.name)
    .bind(params.description)
    .bind(params.system_prompt)
    .bind(params.default_model)
    .bind(params.tools_enabled)
    .bind(params.letta_agent_type)
    .bind(params.letta_tool_ids)
    .bind(id)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("bear not found".to_string()));
    }
    Ok(())
}

/// Creates a logical Bear row. Profile runtime bindings live in `bear_profile_bindings`.
pub async fn create_bear(pool: &PgPool, params: BearParams<'_>) -> Result<Uuid, DenError> {
    create_bear_with_context_profile(pool, params).await
}

/// Creates a logical Bear row with optional profile-aware context composition profile.
pub async fn create_bear_with_context_profile(
    pool: &PgPool,
    params: BearParams<'_>,
) -> Result<Uuid, DenError> {
    let row: (Uuid,) = sqlx::query_as(
        r"
        INSERT INTO bears (
            slug, name, description, system_prompt, default_model, tools_enabled,
            letta_agent_type, letta_tool_ids, context_profile
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id
        ",
    )
    .bind(params.slug)
    .bind(params.name)
    .bind(params.description)
    .bind(params.system_prompt)
    .bind(params.default_model)
    .bind(params.tools_enabled)
    .bind(params.letta_agent_type)
    .bind(params.letta_tool_ids)
    .bind(params.context_profile)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn update_bear_context_profile(
    pool: &PgPool,
    id: Uuid,
    context_profile: Option<Json<serde_json::Value>>,
    system_prompt: &str,
) -> Result<(), DenError> {
    let r = sqlx::query(
        r"
        UPDATE bears
        SET context_profile = $1,
            system_prompt = $2,
            updated_at = NOW()
        WHERE id = $3
        ",
    )
    .bind(context_profile)
    .bind(system_prompt)
    .bind(id)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("bear not found".to_string()));
    }
    Ok(())
}

/// Canonical role for users who manage membership and bear settings (not site `users.is_admin`).
pub const BEAR_ROLE_ADMIN: &str = "admin";
pub const BEAR_ROLE_MEMBER: &str = "member";

#[inline]
pub fn role_is_bear_admin(role: Option<&str>) -> bool {
    matches!(
        role.map(|s| s.trim().eq_ignore_ascii_case(BEAR_ROLE_ADMIN)),
        Some(true)
    )
}

pub async fn grant_membership(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
    role: Option<&str>,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        INSERT INTO user_bear (user_id, bear_id, role)
        VALUES ($1, $2, $3)
        ON CONFLICT (user_id, bear_id) DO UPDATE SET role = EXCLUDED.role
        ",
    )
    .bind(user_id)
    .bind(bear_id)
    .bind(role)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn revoke_membership(pool: &PgPool, user_id: i32, bear_id: Uuid) -> Result<(), DenError> {
    let r = sqlx::query("DELETE FROM user_bear WHERE user_id = $1 AND bear_id = $2")
        .bind(user_id)
        .bind(bear_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("membership not found".to_string()));
    }
    Ok(())
}

pub async fn delete_bear(pool: &PgPool, bear_id: Uuid) -> Result<(), DenError> {
    let r = sqlx::query("DELETE FROM bears WHERE id = $1")
        .bind(bear_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("bear not found".to_string()));
    }
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct BearMemberRow {
    pub user_id: i32,
    pub username: String,
    pub display_name: String,
    pub role: Option<String>,
}

pub async fn list_members_for_bear(
    pool: &PgPool,
    bear_id: Uuid,
) -> Result<Vec<BearMemberRow>, DenError> {
    sqlx::query_as::<_, BearMemberRow>(
        r"
        SELECT ub.user_id, u.username, u.display_name, ub.role
        FROM user_bear ub
        INNER JOIN users u ON u.id = ub.user_id
        WHERE ub.bear_id = $1
        ORDER BY
            CASE WHEN lower(btrim(coalesce(ub.role, ''))) = 'admin' THEN 0 ELSE 1 END,
            u.username
        ",
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn count_bear_admins(pool: &PgPool, bear_id: Uuid) -> Result<i64, DenError> {
    let n: (i64,) = sqlx::query_as(
        r"
        SELECT COUNT(*)::bigint
        FROM user_bear
        WHERE bear_id = $1
          AND lower(btrim(coalesce(role, ''))) = 'admin'
        ",
    )
    .bind(bear_id)
    .fetch_one(pool)
    .await?;
    Ok(n.0)
}

pub async fn membership_role_for_user(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
) -> Result<Option<Option<String>>, DenError> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT role FROM user_bear WHERE user_id = $1 AND bear_id = $2")
            .bind(user_id)
            .bind(bear_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct MembershipRow {
    pub user_id: i32,
    pub username: String,
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub bear_name: String,
    pub role: Option<String>,
}

pub async fn list_memberships(pool: &PgPool) -> Result<Vec<MembershipRow>, DenError> {
    sqlx::query_as::<_, MembershipRow>(
        r"
        SELECT ub.user_id, u.username, ub.bear_id, b.slug AS bear_slug, b.name AS bear_name, ub.role
        FROM user_bear ub
        INNER JOIN users u ON u.id = ub.user_id
        INNER JOIN bears b ON b.id = ub.bear_id
        ORDER BY u.username, b.slug
        ",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn list_bears_for_user(
    pool: &PgPool,
    user_id: i32,
) -> Result<Vec<BearWithMembership>, DenError> {
    sqlx::query_as::<_, BearWithMembership>(
        r"
        SELECT b.id, b.slug, b.name, b.description, b.default_model, b.tools_enabled,
               b.letta_agent_type, b.letta_tool_ids, b.runtime_plan, b.context_profile,
               b.memfs_repo_path, b.provisioning_version, b.system_prompt, b.birthday, b.created_at, b.updated_at,
               ub.role AS membership_role
        FROM bears b
        INNER JOIN user_bear ub ON ub.bear_id = b.id
        WHERE ub.user_id = $1
        ORDER BY b.slug
        ",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Bear visible to the user via `user_bear`, keyed by slug (for `/bear/{slug}`).
pub async fn bear_for_user_by_slug(
    pool: &PgPool,
    user_id: i32,
    slug: &str,
) -> Result<Option<Bear>, DenError> {
    sqlx::query_as::<_, Bear>(
        r"
        SELECT b.id, b.slug, b.name, b.description, b.default_model, b.tools_enabled,
               b.letta_agent_type, b.letta_tool_ids, b.runtime_plan, b.context_profile,
               b.memfs_repo_path, b.provisioning_version, b.system_prompt, b.birthday, b.created_at, b.updated_at
        FROM bears b
        INNER JOIN user_bear ub ON ub.bear_id = b.id
        WHERE ub.user_id = $1 AND b.slug = $2
        ",
    )
    .bind(user_id)
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn count_bear_members(pool: &PgPool, bear_id: Uuid) -> Result<i64, DenError> {
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM user_bear WHERE bear_id = $1")
        .bind(bear_id)
        .fetch_one(pool)
        .await?;
    Ok(n.0)
}

pub async fn user_may_use_bear(
    pool: &PgPool,
    user_id: i32,
    bear_id: Uuid,
) -> Result<bool, DenError> {
    let n: (i64,) = sqlx::query_as(
        r"
        SELECT COUNT(*)::bigint FROM user_bear WHERE user_id = $1 AND bear_id = $2
        ",
    )
    .bind(user_id)
    .bind(bear_id)
    .fetch_one(pool)
    .await?;
    Ok(n.0 > 0)
}

pub async fn ensure_bear_profile_binding_rows(
    pool: &PgPool,
    bear_id: Uuid,
) -> Result<(), DenError> {
    for profile in BearProfile::ALL {
        sqlx::query(
            r"
            INSERT INTO bear_profile_bindings (bear_id, profile, binding_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (bear_id, profile) DO NOTHING
            ",
        )
        .bind(bear_id)
        .bind(profile.as_str())
        .bind(format!("den-native:{bear_id}:{}", profile.as_str()))
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn list_bear_profile_bindings(
    pool: &PgPool,
    bear_id: Uuid,
) -> Result<Vec<BearProfileBinding>, DenError> {
    sqlx::query_as::<_, BearProfileBinding>(
        r"
        SELECT bear_id, profile, binding_id, letta_agent_id, provisioning_status,
               last_provisioned_version, last_synced_at, last_provisioning_error, config_hash,
               created_at, updated_at
        FROM bear_profile_bindings
        WHERE bear_id = $1
        ORDER BY CASE profile
            WHEN 'chat' THEN 1
            WHEN 'pair' THEN 2
            WHEN 'curate' THEN 3
            WHEN 'work' THEN 4
            WHEN 'watch' THEN 5
            ELSE 99
        END
        ",
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn get_bear_profile_binding(
    pool: &PgPool,
    bear_id: Uuid,
    profile: BearProfile,
) -> Result<Option<BearProfileBinding>, DenError> {
    sqlx::query_as::<_, BearProfileBinding>(
        r"
        SELECT bear_id, profile, binding_id, letta_agent_id, provisioning_status,
               last_provisioned_version, last_synced_at, last_provisioning_error, config_hash,
               created_at, updated_at
        FROM bear_profile_bindings
        WHERE bear_id = $1 AND profile = $2
        ",
    )
    .bind(bear_id)
    .bind(profile.as_str())
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

/// Returns the canonical Den-owned runtime binding id for a Bear profile.
pub async fn profile_binding_id(
    pool: &PgPool,
    bear_id: Uuid,
    profile: BearProfile,
) -> Result<Option<String>, DenError> {
    let row: Option<(String,)> = sqlx::query_as(
        r"
        SELECT binding_id
        FROM bear_profile_bindings
        WHERE bear_id = $1 AND profile = $2
        ",
    )
    .bind(bear_id)
    .bind(profile.as_str())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

pub async fn mark_bear_profile_binding_provisioning(
    pool: &PgPool,
    bear_id: Uuid,
    profile: BearProfile,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        INSERT INTO bear_profile_bindings (bear_id, profile, binding_id, provisioning_status, updated_at)
        VALUES ($1, $2, $3, 'provisioning', NOW())
        ON CONFLICT (bear_id, profile)
        DO UPDATE SET provisioning_status = 'provisioning',
                      last_provisioning_error = NULL,
                      updated_at = NOW()
        ",
    )
    .bind(bear_id)
    .bind(profile.as_str())
    .bind(format!("den-native:{bear_id}:{}", profile.as_str()))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_bear_profile_binding_ready(
    pool: &PgPool,
    bear_id: Uuid,
    profile: BearProfile,
    binding_id: &str,
    version: i32,
    config_hash: &serde_json::Value,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        INSERT INTO bear_profile_bindings (
            bear_id, profile, binding_id, letta_agent_id, provisioning_status,
            last_provisioned_version, last_synced_at, last_provisioning_error, config_hash, updated_at
        )
        VALUES ($1, $2, $3, NULL, 'ready', $4, NOW(), NULL, $5::jsonb, NOW())
        ON CONFLICT (bear_id, profile)
        DO UPDATE SET binding_id = EXCLUDED.binding_id,
                      letta_agent_id = NULL,
                      provisioning_status = 'ready',
                      last_provisioned_version = EXCLUDED.last_provisioned_version,
                      last_synced_at = NOW(),
                      last_provisioning_error = NULL,
                      config_hash = EXCLUDED.config_hash,
                      updated_at = NOW()
        ",
    )
    .bind(bear_id)
    .bind(profile.as_str())
    .bind(binding_id)
    .bind(version)
    .bind(config_hash)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_bear_profile_binding_synced(
    pool: &PgPool,
    bear_id: Uuid,
    profile: BearProfile,
    version: i32,
    config_hash: &serde_json::Value,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE bear_profile_bindings
        SET provisioning_status = 'ready',
            last_provisioned_version = $3,
            last_synced_at = NOW(),
            last_provisioning_error = NULL,
            config_hash = $4::jsonb,
            updated_at = NOW()
        WHERE bear_id = $1 AND profile = $2
        ",
    )
    .bind(bear_id)
    .bind(profile.as_str())
    .bind(version)
    .bind(config_hash)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_bear_profile_binding_failed(
    pool: &PgPool,
    bear_id: Uuid,
    profile: BearProfile,
    message: &str,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        INSERT INTO bear_profile_bindings (
            bear_id, profile, binding_id, provisioning_status, last_provisioning_error, updated_at
        )
        VALUES ($1, $2, $3, 'failed', $4, NOW())
        ON CONFLICT (bear_id, profile)
        DO UPDATE SET provisioning_status = 'failed',
                      last_provisioning_error = EXCLUDED.last_provisioning_error,
                      updated_at = NOW()
        ",
    )
    .bind(bear_id)
    .bind(profile.as_str())
    .bind(format!("den-native:{bear_id}:{}", profile.as_str()))
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_bear_skills(
    pool: &PgPool,
    bear_id: Uuid,
) -> Result<Vec<BearSkillManifestEntry>, DenError> {
    sqlx::query_as::<_, BearSkillManifestEntry>(
        r"
        SELECT bear_id, skill_name, skill_version, source, content_hash, applies_to_profiles,
               installed_at, last_verified_at, created_at, updated_at
        FROM bear_skills_manifest
        WHERE bear_id = $1
        ORDER BY skill_name, skill_version
        ",
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn propose_skill(
    pool: &PgPool,
    bear_id: Uuid,
    proposed_by_agent_id: &str,
    skill_payload: &serde_json::Value,
) -> Result<Uuid, DenError> {
    let row: (Uuid,) = sqlx::query_as(
        r"
        INSERT INTO bear_skill_proposals (bear_id, proposed_by_agent_id, skill_payload)
        VALUES ($1, $2, $3::jsonb)
        RETURNING id
        ",
    )
    .bind(bear_id)
    .bind(proposed_by_agent_id)
    .bind(skill_payload)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn list_pending_skill_proposals(
    pool: &PgPool,
    bear_id: Uuid,
) -> Result<Vec<BearSkillProposal>, DenError> {
    sqlx::query_as::<_, BearSkillProposal>(
        r"
        SELECT bear_id, id, proposed_by_agent_id, proposed_at, skill_payload, status,
               reviewed_at, rejection_reason, resulting_manifest_bear_id,
               resulting_manifest_skill_name, resulting_manifest_skill_version, updated_at
        FROM bear_skill_proposals
        WHERE bear_id = $1 AND status = 'pending_review'
        ORDER BY proposed_at, id
        ",
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Seed `runtime_plan` once so codepool always has a BearRuntimePlan v1 snapshot.
pub async fn ensure_default_runtime_plan(
    pool: &PgPool,
    bear_id: Uuid,
    default_json: &serde_json::Value,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        UPDATE bears
        SET runtime_plan = $2::jsonb,
            updated_at = NOW()
        WHERE id = $1
          AND runtime_plan IS NULL
        ",
    )
    .bind(bear_id)
    .bind(default_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_profile_model_settings(
    pool: &PgPool,
    bear_id: Uuid,
) -> Result<Vec<BearProfileModelSetting>, DenError> {
    sqlx::query_as::<_, BearProfileModelSetting>(
        r#"
        SELECT bear_id, profile, model
        FROM bear_profile_model_settings
        WHERE bear_id = $1
        ORDER BY profile
        "#,
    )
    .bind(bear_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn profile_model_setting(
    pool: &PgPool,
    bear_id: Uuid,
    profile: BearProfile,
) -> Result<Option<String>, DenError> {
    sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT model
        FROM bear_profile_model_settings
        WHERE bear_id = $1 AND profile = $2
        "#,
    )
    .bind(bear_id)
    .bind(profile.as_str())
    .fetch_optional(pool)
    .await
    .map(|row| row.flatten().and_then(|model| {
        let trimmed = model.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }))
    .map_err(Into::into)
}

pub async fn set_profile_model_setting(
    pool: &PgPool,
    bear_id: Uuid,
    profile: BearProfile,
    model: Option<&str>,
) -> Result<(), DenError> {
    let model = model.map(str::trim).filter(|s| !s.is_empty());
    sqlx::query(
        r#"
        INSERT INTO bear_profile_model_settings (bear_id, profile, model, updated_at)
        VALUES ($1, $2, $3, NOW())
        ON CONFLICT (bear_id, profile) DO UPDATE
        SET model = EXCLUDED.model,
            updated_at = NOW()
        "#,
    )
    .bind(bear_id)
    .bind(profile.as_str())
    .bind(model)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn resolve_model_for_profile(
    pool: &PgPool,
    bear: &Bear,
    profile: BearProfile,
    system_default_model: &str,
) -> Result<String, DenError> {
    if let Some(model) = profile_model_setting(pool, bear.id, profile).await? {
        return Ok(model);
    }
    if let Some(model) = bear.default_model.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(model.to_string());
    }
    Ok(system_default_model.trim().to_string())
}
