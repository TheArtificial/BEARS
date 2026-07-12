//! Managed work surfaces and the sandbox image catalog.
//!
//! A work surface is a den-level entity backing a sandbox root: a git
//! upstream plus an optional access credential, created by a user (the
//! owner), manageable by granted users, and assigned to bears (full access).
//! Den's database is the source of truth; [`build_managed_config`] produces
//! the declarative payload pushed to the sandbox provider.
//!
//! Not to be confused with `den-core`'s `tools::work_surface` (memory
//! scaffolding) — this module is about sandbox provisioning surfaces.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::DenError;
use den_sandbox::protocol::{ManagedConfig, ManagedCredential, ManagedImage, ManagedSurface};

pub const SURFACE_ROLE_OWNER: &str = "owner";
pub const SURFACE_ROLE_MANAGER: &str = "manager";

pub const CREDENTIAL_KIND_SSH_KEY: &str = "ssh_key";
pub const CREDENTIAL_KIND_HTTPS_TOKEN: &str = "https_token";

/// Matches the DB CHECK constraint on surface and catalog names. Names are
/// used as provider-side directory components, so path characters are out.
pub fn name_is_valid(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

fn validate_name(name: &str) -> Result<(), DenError> {
    if name_is_valid(name) {
        Ok(())
    } else {
        Err(DenError::ValidationError(format!(
            "invalid surface name '{name}': lowercase letters, digits, '.', '_', '-' only (max 64 chars, must start with a letter or digit)"
        )))
    }
}

fn validate_credential_kind(kind: &str) -> Result<(), DenError> {
    match kind {
        CREDENTIAL_KIND_SSH_KEY | CREDENTIAL_KIND_HTTPS_TOKEN => Ok(()),
        other => Err(DenError::ValidationError(format!(
            "unknown credential kind '{other}' (expected 'ssh_key' or 'https_token')"
        ))),
    }
}

/// A work surface without its credential ciphertext. This is the only row
/// shape general code sees; `credential_kind` doubles as the "credential is
/// set" signal. Ciphertexts are read exclusively by [`build_managed_config`].
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct WorkSurfaceRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub upstream_url: String,
    pub default_ref: String,
    pub default_image: Option<String>,
    pub credential_kind: Option<String>,
    pub created_by_user_id: i32,
    pub created_at: time::OffsetDateTime,
    pub updated_at: time::OffsetDateTime,
}

const SURFACE_COLUMNS: &str = "id, name, description, upstream_url, default_ref, default_image, \
     credential_kind, created_by_user_id, created_at, updated_at";

#[derive(Debug, Clone)]
pub struct NewWorkSurface {
    pub name: String,
    pub description: Option<String>,
    pub upstream_url: String,
    pub default_ref: String,
    pub default_image: Option<String>,
    /// (kind, plaintext value); encrypted before it reaches the database.
    pub credential: Option<(String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct WorkSurfaceUpdate {
    pub description: Option<Option<String>>,
    pub upstream_url: Option<String>,
    pub default_ref: Option<String>,
    pub default_image: Option<Option<String>>,
}

pub async fn create_surface(
    pool: &PgPool,
    owner_user_id: i32,
    surface: NewWorkSurface,
    secret_key: &str,
) -> Result<WorkSurfaceRow, DenError> {
    validate_name(&surface.name)?;
    if surface.upstream_url.trim().is_empty() {
        return Err(DenError::ValidationError(
            "upstream URL must not be empty".to_string(),
        ));
    }
    let (credential_kind, credential_encrypted) = match &surface.credential {
        Some((kind, value)) => {
            validate_credential_kind(kind)?;
            (
                Some(kind.clone()),
                Some(crate::secrets::encrypt_secret(value, secret_key)?),
            )
        }
        None => (None, None),
    };

    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<_, WorkSurfaceRow>(&format!(
        r"
        INSERT INTO work_surfaces
            (name, description, upstream_url, default_ref, default_image,
             credential_kind, credential_encrypted, created_by_user_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING {SURFACE_COLUMNS}
        ",
    ))
    .bind(&surface.name)
    .bind(&surface.description)
    .bind(surface.upstream_url.trim())
    .bind(surface.default_ref.trim())
    .bind(&surface.default_image)
    .bind(&credential_kind)
    .bind(&credential_encrypted)
    .bind(owner_user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|err| match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("work_surfaces_name_key") => {
            DenError::ValidationError(format!(
                "a work surface named '{}' already exists",
                surface.name
            ))
        }
        _ => DenError::from(err),
    })?;
    sqlx::query(
        r"
        INSERT INTO work_surface_managers (surface_id, user_id, role, granted_by_user_id)
        VALUES ($1, $2, 'owner', $2)
        ",
    )
    .bind(row.id)
    .bind(owner_user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Update mutable fields. The name is deliberately not updatable: it is the
/// provider-side root identity and is denormalized into job refs.
pub async fn update_surface(
    pool: &PgPool,
    surface_id: Uuid,
    update: WorkSurfaceUpdate,
) -> Result<WorkSurfaceRow, DenError> {
    if let Some(url) = &update.upstream_url {
        if url.trim().is_empty() {
            return Err(DenError::ValidationError(
                "upstream URL must not be empty".to_string(),
            ));
        }
    }
    if let Some(default_ref) = &update.default_ref {
        if default_ref.trim().is_empty() {
            return Err(DenError::ValidationError(
                "default ref must not be empty".to_string(),
            ));
        }
    }
    sqlx::query_as::<_, WorkSurfaceRow>(&format!(
        r"
        UPDATE work_surfaces SET
            description = CASE WHEN $2 THEN $3 ELSE description END,
            upstream_url = COALESCE($4, upstream_url),
            default_ref = COALESCE($5, default_ref),
            default_image = CASE WHEN $6 THEN $7 ELSE default_image END,
            updated_at = now()
        WHERE id = $1
        RETURNING {SURFACE_COLUMNS}
        ",
    ))
    .bind(surface_id)
    .bind(update.description.is_some())
    .bind(update.description.flatten())
    .bind(update.upstream_url.as_deref().map(str::trim))
    .bind(update.default_ref.as_deref().map(str::trim))
    .bind(update.default_image.is_some())
    .bind(update.default_image.flatten())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DenError::NotFound("work surface not found".to_string()))
}

pub async fn set_credential(
    pool: &PgPool,
    surface_id: Uuid,
    kind: &str,
    value: &str,
    secret_key: &str,
) -> Result<(), DenError> {
    validate_credential_kind(kind)?;
    let encrypted = crate::secrets::encrypt_secret(value, secret_key)?;
    let r = sqlx::query(
        r"
        UPDATE work_surfaces
        SET credential_kind = $2, credential_encrypted = $3, updated_at = now()
        WHERE id = $1
        ",
    )
    .bind(surface_id)
    .bind(kind)
    .bind(&encrypted)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("work surface not found".to_string()));
    }
    Ok(())
}

pub async fn clear_credential(pool: &PgPool, surface_id: Uuid) -> Result<(), DenError> {
    let r = sqlx::query(
        r"
        UPDATE work_surfaces
        SET credential_kind = NULL, credential_encrypted = NULL, updated_at = now()
        WHERE id = $1
        ",
    )
    .bind(surface_id)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("work surface not found".to_string()));
    }
    Ok(())
}

pub async fn delete_surface(pool: &PgPool, surface_id: Uuid) -> Result<(), DenError> {
    let r = sqlx::query("DELETE FROM work_surfaces WHERE id = $1")
        .bind(surface_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("work surface not found".to_string()));
    }
    Ok(())
}

pub async fn surface_by_id(
    pool: &PgPool,
    surface_id: Uuid,
) -> Result<Option<WorkSurfaceRow>, DenError> {
    Ok(sqlx::query_as::<_, WorkSurfaceRow>(&format!(
        "SELECT {SURFACE_COLUMNS} FROM work_surfaces WHERE id = $1",
    ))
    .bind(surface_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn surface_by_name(
    pool: &PgPool,
    name: &str,
) -> Result<Option<WorkSurfaceRow>, DenError> {
    Ok(sqlx::query_as::<_, WorkSurfaceRow>(&format!(
        "SELECT {SURFACE_COLUMNS} FROM work_surfaces WHERE name = $1",
    ))
    .bind(name)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_all_surfaces(pool: &PgPool) -> Result<Vec<WorkSurfaceRow>, DenError> {
    Ok(sqlx::query_as::<_, WorkSurfaceRow>(&format!(
        "SELECT {SURFACE_COLUMNS} FROM work_surfaces ORDER BY name",
    ))
    .fetch_all(pool)
    .await?)
}

pub async fn list_surfaces_managed_by(
    pool: &PgPool,
    user_id: i32,
) -> Result<Vec<WorkSurfaceRow>, DenError> {
    Ok(sqlx::query_as::<_, WorkSurfaceRow>(&format!(
        r"
        SELECT {SURFACE_COLUMNS} FROM work_surfaces s
        WHERE EXISTS (
            SELECT 1 FROM work_surface_managers m
            WHERE m.surface_id = s.id AND m.user_id = $1
        )
        ORDER BY s.name
        ",
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

pub async fn user_may_manage_surface(
    pool: &PgPool,
    user_id: i32,
    surface_id: Uuid,
) -> Result<bool, DenError> {
    let exists: bool = sqlx::query_scalar(
        r"
        SELECT EXISTS (
            SELECT 1 FROM work_surface_managers
            WHERE surface_id = $1 AND user_id = $2
        )
        ",
    )
    .bind(surface_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

/// Surfaces assigned to any of the given bears, with the bears each is
/// assigned to (for grouping in dispatch UIs).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SurfaceForBearRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub upstream_url: String,
    pub default_ref: String,
    pub default_image: Option<String>,
    pub bear_id: Uuid,
}

pub async fn list_surfaces_for_bears(
    pool: &PgPool,
    bear_ids: &[Uuid],
) -> Result<Vec<SurfaceForBearRow>, DenError> {
    Ok(sqlx::query_as::<_, SurfaceForBearRow>(
        r"
        SELECT s.id, s.name, s.description, s.upstream_url, s.default_ref,
               s.default_image, sb.bear_id
        FROM work_surfaces s
        INNER JOIN work_surface_bears sb ON sb.surface_id = s.id
        WHERE sb.bear_id = ANY($1)
        ORDER BY s.name, sb.bear_id
        ",
    )
    .bind(bear_ids)
    .fetch_all(pool)
    .await?)
}

pub async fn bear_may_use_surface(
    pool: &PgPool,
    bear_id: Uuid,
    surface_id: Uuid,
) -> Result<bool, DenError> {
    let exists: bool = sqlx::query_scalar(
        r"
        SELECT EXISTS (
            SELECT 1 FROM work_surface_bears
            WHERE surface_id = $1 AND bear_id = $2
        )
        ",
    )
    .bind(surface_id)
    .bind(bear_id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SurfaceManagerRow {
    pub user_id: i32,
    pub username: String,
    pub display_name: String,
    pub role: String,
}

pub async fn list_managers(
    pool: &PgPool,
    surface_id: Uuid,
) -> Result<Vec<SurfaceManagerRow>, DenError> {
    Ok(sqlx::query_as::<_, SurfaceManagerRow>(
        r"
        SELECT m.user_id, u.username, u.display_name, m.role
        FROM work_surface_managers m
        INNER JOIN users u ON u.id = m.user_id
        WHERE m.surface_id = $1
        ORDER BY CASE WHEN m.role = 'owner' THEN 0 ELSE 1 END, u.username
        ",
    )
    .bind(surface_id)
    .fetch_all(pool)
    .await?)
}

pub async fn grant_manager(
    pool: &PgPool,
    surface_id: Uuid,
    user_id: i32,
    role: &str,
    granted_by: i32,
) -> Result<(), DenError> {
    if role != SURFACE_ROLE_OWNER && role != SURFACE_ROLE_MANAGER {
        return Err(DenError::ValidationError(format!(
            "unknown surface role '{role}'"
        )));
    }
    sqlx::query(
        r"
        INSERT INTO work_surface_managers (surface_id, user_id, role, granted_by_user_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (surface_id, user_id) DO UPDATE SET role = EXCLUDED.role
        ",
    )
    .bind(surface_id)
    .bind(user_id)
    .bind(role)
    .bind(granted_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Revoke a manager. Refuses to remove the last owner so a surface can never
/// become unmanageable (site admins bypass grants entirely).
pub async fn revoke_manager(pool: &PgPool, surface_id: Uuid, user_id: i32) -> Result<(), DenError> {
    let mut tx = pool.begin().await?;
    let role: Option<String> = sqlx::query_scalar(
        r"
        SELECT role FROM work_surface_managers
        WHERE surface_id = $1 AND user_id = $2
        FOR UPDATE
        ",
    )
    .bind(surface_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(role) = role else {
        return Err(DenError::NotFound("manager grant not found".to_string()));
    };
    if role == SURFACE_ROLE_OWNER {
        let owners: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM work_surface_managers WHERE surface_id = $1 AND role = 'owner'",
        )
        .bind(surface_id)
        .fetch_one(&mut *tx)
        .await?;
        if owners <= 1 {
            return Err(DenError::ValidationError(
                "cannot remove the last owner of a work surface".to_string(),
            ));
        }
    }
    sqlx::query("DELETE FROM work_surface_managers WHERE surface_id = $1 AND user_id = $2")
        .bind(surface_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct AssignedBearRow {
    pub bear_id: Uuid,
    pub slug: String,
    pub display_name: String,
}

pub async fn list_assigned_bears(
    pool: &PgPool,
    surface_id: Uuid,
) -> Result<Vec<AssignedBearRow>, DenError> {
    Ok(sqlx::query_as::<_, AssignedBearRow>(
        r"
        SELECT sb.bear_id, b.slug, b.name AS display_name
        FROM work_surface_bears sb
        INNER JOIN bears b ON b.id = sb.bear_id
        WHERE sb.surface_id = $1
        ORDER BY b.slug
        ",
    )
    .bind(surface_id)
    .fetch_all(pool)
    .await?)
}

pub async fn assign_bear(
    pool: &PgPool,
    surface_id: Uuid,
    bear_id: Uuid,
    granted_by: i32,
) -> Result<(), DenError> {
    sqlx::query(
        r"
        INSERT INTO work_surface_bears (surface_id, bear_id, granted_by_user_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (surface_id, bear_id) DO NOTHING
        ",
    )
    .bind(surface_id)
    .bind(bear_id)
    .bind(granted_by)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unassign_bear(pool: &PgPool, surface_id: Uuid, bear_id: Uuid) -> Result<(), DenError> {
    let r = sqlx::query("DELETE FROM work_surface_bears WHERE surface_id = $1 AND bear_id = $2")
        .bind(surface_id)
        .bind(bear_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("bear assignment not found".to_string()));
    }
    Ok(())
}

// --- catalog images ---

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CatalogImageRow {
    pub id: Uuid,
    pub name: String,
    pub image_ref: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub created_at: time::OffsetDateTime,
    pub updated_at: time::OffsetDateTime,
}

pub async fn list_catalog_images(pool: &PgPool) -> Result<Vec<CatalogImageRow>, DenError> {
    Ok(sqlx::query_as::<_, CatalogImageRow>(
        r"
        SELECT id, name, image_ref, description, is_default, created_at, updated_at
        FROM sandbox_catalog_images
        ORDER BY name
        ",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn create_catalog_image(
    pool: &PgPool,
    name: &str,
    image_ref: &str,
    description: Option<&str>,
    created_by: i32,
) -> Result<CatalogImageRow, DenError> {
    validate_name(name)?;
    if image_ref.trim().is_empty() {
        return Err(DenError::ValidationError(
            "image reference must not be empty".to_string(),
        ));
    }
    sqlx::query_as::<_, CatalogImageRow>(
        r"
        INSERT INTO sandbox_catalog_images (name, image_ref, description, created_by_user_id)
        VALUES ($1, $2, $3, $4)
        RETURNING id, name, image_ref, description, is_default, created_at, updated_at
        ",
    )
    .bind(name)
    .bind(image_ref.trim())
    .bind(description)
    .bind(created_by)
    .fetch_one(pool)
    .await
    .map_err(|err| match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("sandbox_catalog_images_name_key") => {
            DenError::ValidationError(format!("a catalog image named '{name}' already exists"))
        }
        _ => DenError::from(err),
    })
}

pub async fn update_catalog_image(
    pool: &PgPool,
    image_id: Uuid,
    image_ref: &str,
    description: Option<&str>,
) -> Result<(), DenError> {
    if image_ref.trim().is_empty() {
        return Err(DenError::ValidationError(
            "image reference must not be empty".to_string(),
        ));
    }
    let r = sqlx::query(
        r"
        UPDATE sandbox_catalog_images
        SET image_ref = $2, description = $3, updated_at = now()
        WHERE id = $1
        ",
    )
    .bind(image_id)
    .bind(image_ref.trim())
    .bind(description)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("catalog image not found".to_string()));
    }
    Ok(())
}

pub async fn delete_catalog_image(pool: &PgPool, image_id: Uuid) -> Result<(), DenError> {
    let r = sqlx::query("DELETE FROM sandbox_catalog_images WHERE id = $1")
        .bind(image_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("catalog image not found".to_string()));
    }
    Ok(())
}

pub async fn set_default_catalog_image(pool: &PgPool, image_id: Uuid) -> Result<(), DenError> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE sandbox_catalog_images SET is_default = FALSE WHERE is_default")
        .execute(&mut *tx)
        .await?;
    let r = sqlx::query(
        "UPDATE sandbox_catalog_images SET is_default = TRUE, updated_at = now() WHERE id = $1",
    )
    .bind(image_id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DenError::NotFound("catalog image not found".to_string()));
    }
    tx.commit().await?;
    Ok(())
}

// --- provider sync payload ---

#[derive(sqlx::FromRow)]
struct SurfaceSyncRow {
    name: String,
    upstream_url: String,
    default_ref: String,
    default_image: Option<String>,
    credential_kind: Option<String>,
    credential_encrypted: Option<String>,
}

/// Build the declarative managed-config payload for the sandbox provider.
/// This is the single place credential ciphertexts are read and decrypted.
/// The `version` hashes ciphertexts (never plaintexts) plus all non-secret
/// fields, so credential rotation changes it without exposing key material.
pub async fn build_managed_config(
    pool: &PgPool,
    secret_key: &str,
) -> Result<ManagedConfig, DenError> {
    let surface_rows = sqlx::query_as::<_, SurfaceSyncRow>(
        r"
        SELECT name, upstream_url, default_ref, default_image,
               credential_kind, credential_encrypted
        FROM work_surfaces
        ORDER BY name
        ",
    )
    .fetch_all(pool)
    .await?;
    let images = list_catalog_images(pool).await?;

    let mut hasher = Sha256::new();
    for row in &surface_rows {
        hasher.update(row.name.as_bytes());
        hasher.update([0]);
        hasher.update(row.upstream_url.as_bytes());
        hasher.update([0]);
        hasher.update(row.default_ref.as_bytes());
        hasher.update([0]);
        hasher.update(row.default_image.as_deref().unwrap_or("").as_bytes());
        hasher.update([0]);
        hasher.update(row.credential_kind.as_deref().unwrap_or("").as_bytes());
        hasher.update([0]);
        hasher.update(row.credential_encrypted.as_deref().unwrap_or("").as_bytes());
        hasher.update([1]);
    }
    for image in &images {
        hasher.update(image.name.as_bytes());
        hasher.update([0]);
        hasher.update(image.image_ref.as_bytes());
        hasher.update([0]);
        hasher.update(image.description.as_deref().unwrap_or("").as_bytes());
        hasher.update([0]);
        hasher.update([u8::from(image.is_default)]);
        hasher.update([1]);
    }
    let version = format!("{:x}", hasher.finalize());

    let mut surfaces = Vec::with_capacity(surface_rows.len());
    for row in surface_rows {
        let credential = match (row.credential_kind.as_deref(), &row.credential_encrypted) {
            (Some(CREDENTIAL_KIND_SSH_KEY), Some(encrypted)) => Some(ManagedCredential::SshKey {
                private_key: crate::secrets::decrypt_secret(encrypted, secret_key)?,
            }),
            (Some(CREDENTIAL_KIND_HTTPS_TOKEN), Some(encrypted)) => {
                Some(ManagedCredential::HttpsToken {
                    token: crate::secrets::decrypt_secret(encrypted, secret_key)?,
                })
            }
            _ => None,
        };
        surfaces.push(ManagedSurface {
            name: row.name,
            upstream_url: row.upstream_url,
            default_ref: row.default_ref,
            default_image: row.default_image,
            credential,
        });
    }
    let images = images
        .into_iter()
        .map(|image| ManagedImage {
            name: image.name,
            image: image.image_ref,
            description: image.description,
            default: image.is_default,
        })
        .collect();

    Ok(ManagedConfig {
        surfaces,
        images,
        version: Some(version),
    })
}

#[cfg(test)]
mod tests;
