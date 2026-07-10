//! Push the Den-managed sandbox configuration (work surfaces + image
//! catalog) to the sandbox provider.
//!
//! Den's Postgres is the source of truth; the provider persists the pushed
//! set on its volume as a cache. Sync triggers: a forced push on work-
//! dispatch worker startup, a periodic version-checked reconcile on the
//! dispatch tick, and best-effort pushes from web handlers after mutations.
//! Every trigger is idempotent (declarative full-set PUT), so a missed push
//! self-heals at the next one.

use sqlx::PgPool;

use den_core::config::Config;
use den_core::DenError;
use den_sandbox::SandboxClient;
use den_service::work_surfaces::build_managed_config;

/// Build the current managed config and push it, unconditionally.
pub async fn push_managed_config(
    pool: &PgPool,
    config: &Config,
    client: &SandboxClient,
) -> Result<(), DenError> {
    let managed = build_managed_config(pool, &config.den_secret_encryption_key).await?;
    let status = client
        .put_managed_config(&managed)
        .await
        .map_err(|err| DenError::System(format!("push managed config: {err}")))?;
    tracing::info!(
        surfaces = status.surfaces,
        images = status.images,
        version = status.version.as_deref().unwrap_or("-"),
        "surface_sync: managed config pushed to sandbox provider"
    );
    Ok(())
}

/// Push only when the provider's applied version differs from ours (or when
/// `force` is set). Cheap when in sync: one GET, no decryption payload sent.
pub async fn reconcile_if_stale(
    pool: &PgPool,
    config: &Config,
    client: &SandboxClient,
    force: bool,
) -> Result<(), DenError> {
    if !force {
        let ours = build_managed_config(pool, &config.den_secret_encryption_key)
            .await?
            .version;
        let theirs = client
            .managed_config_status()
            .await
            .map_err(|err| DenError::System(format!("read managed config status: {err}")))?
            .version;
        if ours == theirs {
            return Ok(());
        }
        tracing::info!(
            ours = ours.as_deref().unwrap_or("-"),
            theirs = theirs.as_deref().unwrap_or("-"),
            "surface_sync: provider managed config is stale; pushing"
        );
    }
    push_managed_config(pool, config, client).await
}
