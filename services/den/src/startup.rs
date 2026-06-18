//! Startup validation, SQLx migration runner, and structured errors for [`crate::run`].

use crate::config::Config;
use den_runtime::runtime_provider::{acp_requires_runtime, RuntimeStartupCapabilities};
use sqlx::PgPool;
use thiserror::Error;

/// Failures while initializing the process (config, database, migrations, tracing).
#[derive(Debug, Error)]
pub enum StartupError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// Tower-sessions Postgres schema migration (error type varies by store version).
    #[error("tower-sessions store migration: {0}")]
    SessionStore(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("tracing subscriber: {0}")]
    Tracing(String),
    /// Database connection failed with operator-actionable context.
    #[error("database error: {message} (url={db_url})\n  hint: {hint}")]
    Database {
        message: String,
        db_url: String,
        hint: String,
    },
}

/// `true` when SQLx should ignore migration files present in `_sqlx_migrations` but missing
/// from the embedded migrator (recovery / legacy DBs only). Default **false** — set
/// `SQLX_MIGRATE_IGNORE_MISSING=true` only when you understand the risk.
pub fn sqlx_migrate_ignore_missing_from_env() -> bool {
    std::env::var("SQLX_MIGRATE_IGNORE_MISSING")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Run embedded SQLx migrations from `migrations/` against `pool`.
pub async fn run_sqlx_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    if sqlx_migrate_ignore_missing_from_env() {
        sqlx::migrate!().set_ignore_missing(true).run(pool).await
    } else {
        sqlx::migrate!().run(pool).await
    }
}

/// Whether [`validate_runtime_config`] requires a non-empty `JWT_SECRET` (production builds or `RUN_API`).
///
/// Canonical home is `den_core::config`; re-exported so the web edge (den-web) and
/// the binary's startup validation share one definition.
pub use den_core::config::requires_jwt_secret;

/// Validate secrets and other invariants before connecting to the database.
pub fn validate_runtime_config(config: &Config) -> Result<(), StartupError> {
    if requires_jwt_secret(config) {
        let secret = std::env::var("JWT_SECRET").unwrap_or_default();
        if secret.trim().is_empty() {
            return Err(StartupError::Message(
                "JWT_SECRET must be set to a non-empty value when the binary is built with \
                 `--features production`, or when RUN_API=true (OAuth access tokens use HS256)."
                    .into(),
            ));
        }
    }
    if config.acp_gateway_enabled && !config.run_api {
        return Err(StartupError::Message(
            "ACP_GATEWAY_ENABLED=true requires RUN_API=true because ACP is exposed only on the API listener."
                .into(),
        ));
    }
    if acp_requires_runtime(config) && config.llm_api_url.trim().is_empty() {
        return Err(StartupError::Message(
            "LLM_API_URL (or BIFROST_BASE_URL) must be set when ACP_GATEWAY_ENABLED=true (Den-native agent loop)."
                .into(),
        ));
    }
    Ok(())
}

/// Verify configured upstream HTTP services respond before accepting traffic.
///
/// The Den-native runtime relies only on the LLM inference substrate
/// ([`Config::llm_api_url`], via Bifrost); there are no Letta/Codepool sidecars to probe.
pub async fn validate_upstream_connections(config: &Config) -> Result<(), StartupError> {
    let runtime_capabilities = RuntimeStartupCapabilities::from_config(config);
    if !config.llm_api_url.trim().is_empty() {
        tracing::info!(
            url = %config.llm_api_url,
            compatibility_backend = "native",
            acp_gateway_enabled = runtime_capabilities.acp_gateway_enabled,
            "Native agent runtime configured (LLM inference substrate)"
        );
    }

    bootstrap_recall_index(config).await;

    Ok(())
}

/// Best-effort bootstrap of the derived recall index (Qdrant). Recall is optional and
/// derived from the canonical SQLite store, so failures here are logged and swallowed —
/// Den degrades to keyword fallback rather than failing startup (ADR-0038).
async fn bootstrap_recall_index(config: &Config) {
    let Some(recall) = den_runtime::recall::QdrantRecall::from_config(config) else {
        return;
    };
    match recall.ensure_collection().await {
        Ok(created) => tracing::info!(
            collection = recall.collection_name(),
            created,
            url = recall.base_url(),
            "Derived recall index (Qdrant) ready"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            collection = recall.collection_name(),
            url = recall.base_url(),
            "Qdrant recall index unavailable at startup; continuing with keyword fallback"
        ),
    }
}

#[cfg(all(test, not(feature = "production")))]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn validate_jwt_rules_when_not_production_build() {
        let prev = std::env::var("JWT_SECRET").ok();
        // SAFETY: single-threaded test; no concurrent env reads in this process.
        unsafe {
            std::env::remove_var("JWT_SECRET");
        }

        let base = Config::test_stub();
        validate_runtime_config(&base).expect("web-only must not require JWT_SECRET");

        let mut api_on = base;
        api_on.run_api = true;
        assert!(
            validate_runtime_config(&api_on).is_err(),
            "RUN_API=true requires JWT_SECRET"
        );

        unsafe {
            std::env::set_var("JWT_SECRET", "test-jwt-secret-for-unit-tests-min-length-ok");
        }
        validate_runtime_config(&api_on).expect("RUN_API with JWT_SECRET should pass");

        match prev {
            Some(v) => unsafe { std::env::set_var("JWT_SECRET", v) },
            None => unsafe { std::env::remove_var("JWT_SECRET") },
        }
    }

    #[test]
    fn validate_requires_api_and_llm_when_acp_enabled() {
        let prev = std::env::var("JWT_SECRET").ok();
        unsafe {
            std::env::set_var("JWT_SECRET", "test-jwt-secret-for-unit-tests-min-length-ok");
        }

        let mut acp_on = Config::test_stub();
        acp_on.acp_gateway_enabled = true;
        assert!(
            validate_runtime_config(&acp_on).is_err(),
            "ACP_GATEWAY_ENABLED requires RUN_API"
        );

        acp_on.run_api = true;
        assert!(
            validate_runtime_config(&acp_on).is_err(),
            "ACP runtime requires LLM_API_URL"
        );

        acp_on.llm_api_url = "http://bears-bifrost:8080/v1".into();
        validate_runtime_config(&acp_on).expect("ACP with API and LLM_API_URL should pass");

        match prev {
            Some(v) => unsafe { std::env::set_var("JWT_SECRET", v) },
            None => unsafe { std::env::remove_var("JWT_SECRET") },
        }
    }

    #[test]
    fn validate_allows_run_web_without_legacy_runtime() {
        let mut web_on = Config::test_stub();
        web_on.run_web = true;
        validate_runtime_config(&web_on).expect("RUN_WEB has no legacy runtime requirement");
    }
}
