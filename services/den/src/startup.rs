//! Startup validation, SQLx migration runner, and structured errors for [`crate::run`].

use crate::config::Config;
use crate::core::codepool::CodePoolClient;
use crate::core::letta::LettaClient;
use crate::core::runtime_provider::{
    acp_requires_runtime, RuntimeHealthCheck, RuntimeStartupCapabilities,
};
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
pub fn requires_jwt_secret(config: &Config) -> bool {
    #[cfg(feature = "production")]
    {
        let _ = config;
        true
    }
    #[cfg(not(feature = "production"))]
    {
        config.run_api
    }
}

fn allow_standalone_web_from_env() -> bool {
    std::env::var("DEN_ALLOW_STANDALONE_WEB")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

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
    if acp_requires_runtime(config)
        && config.letta_base_url.trim().is_empty()
        && !config.uses_native_agent_runtime()
    {
        return Err(StartupError::Message(
            "LETTA_BASE_URL must be set when ACP_GATEWAY_ENABLED=true and AGENT_RUNTIME=letta. \
             Set AGENT_RUNTIME=native to use the Den in-process agent loop with LLM_API_URL."
                .into(),
        ));
    }
    if acp_requires_runtime(config)
        && config.uses_native_agent_runtime()
        && config.llm_api_url.trim().is_empty()
    {
        return Err(StartupError::Message(
            "LLM_API_URL (or BIFROST_BASE_URL) must be set when ACP_GATEWAY_ENABLED=true and AGENT_RUNTIME=native."
                .into(),
        ));
    }
    if config.run_web
        && config.codepool_base_url.trim().is_empty()
        && !config.uses_native_agent_runtime()
        && !allow_standalone_web_from_env()
    {
        return Err(StartupError::Message(
            "CODEPOOL_BASE_URL must be set when RUN_WEB=true and AGENT_RUNTIME=letta. \
             Set AGENT_RUNTIME=native for the Den in-process runtime, or \
             DEN_ALLOW_STANDALONE_WEB=true for local UI-only runs."
                .into(),
        ));
    }
    if !config.uses_native_agent_runtime() {
        tracing::warn!(
            "AGENT_RUNTIME=letta is deprecated and will be removed in Phase 8; use AGENT_RUNTIME=native"
        );
    }
    Ok(())
}

/// Verify configured upstream HTTP services respond before accepting traffic.
///
/// Checks **Codepool** (`GET /health`) when [`Config::codepool_base_url`] is non-empty,
/// and **Letta** (`GET /v1/health`) when [`Config::letta_base_url`] is non-empty (same auth as runtime).
pub async fn validate_upstream_connections(config: &Config) -> Result<(), StartupError> {
    if !config.uses_native_agent_runtime() && !config.codepool_base_url.trim().is_empty() {
        tracing::info!(
            url = %config.codepool_base_url,
            "Checking Codepool connectivity"
        );
        CodePoolClient::new(config)
            .check_health()
            .await
            .map_err(|e| StartupError::Message(e.to_string()))?;
        tracing::info!("Codepool health check passed");
    } else if config.uses_native_agent_runtime() {
        tracing::info!("Skipping Codepool health check (AGENT_RUNTIME=native)");
    }

    let runtime_capabilities = RuntimeStartupCapabilities::from_config(config);
    if config.uses_native_agent_runtime() && !config.llm_api_url.trim().is_empty() {
        tracing::info!(
            url = %config.llm_api_url,
            compatibility_backend = "native",
            acp_gateway_enabled = runtime_capabilities.acp_gateway_enabled,
            "Native agent runtime configured (LLM inference substrate)"
        );
    }
    if runtime_capabilities.runtime_required_for_acp || !config.letta_base_url.trim().is_empty() {
        if config.uses_native_agent_runtime() {
            tracing::info!("Skipping Letta health check (AGENT_RUNTIME=native)");
        } else {
        tracing::info!(
            url = %config.letta_base_url,
            compatibility_backend = "letta",
            acp_gateway_enabled = runtime_capabilities.acp_gateway_enabled,
            "Checking runtime compatibility backend connectivity"
        );
        let letta = LettaClient::new(config);
        let health = letta.compatibility_health_check();
        if health.enabled() {
            RuntimeHealthCheck::check_health(&health)
                .await
                .map_err(|e| StartupError::Message(e.to_string()))?;
            tracing::info!(
                compatibility_backend = RuntimeHealthCheck::compatibility_backend_name(&health),
                "Runtime compatibility backend health check passed"
            );
        } else if runtime_capabilities.runtime_required_for_acp
            && !config.uses_native_agent_runtime()
        {
            return Err(StartupError::Message(
                "LETTA_BASE_URL must be set when ACP_GATEWAY_ENABLED=true and AGENT_RUNTIME=letta."
                    .into(),
            ));
        }
        }
    }

    Ok(())
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

        let mut api_on = base.clone();
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
    fn validate_requires_api_and_letta_when_acp_enabled() {
        let prev = std::env::var("JWT_SECRET").ok();
        unsafe {
            std::env::set_var("JWT_SECRET", "test-jwt-secret-for-unit-tests-min-length-ok");
        }

        let mut acp_on = Config::test_stub();
        acp_on.acp_gateway_enabled = true;
        assert!(validate_runtime_config(&acp_on).is_err());

        acp_on.run_api = true;
        assert!(validate_runtime_config(&acp_on).is_err());

        acp_on.letta_base_url = "http://bears-letta:8283".into();
        validate_runtime_config(&acp_on).expect("ACP with API and Letta should pass");

        match prev {
            Some(v) => unsafe { std::env::set_var("JWT_SECRET", v) },
            None => unsafe { std::env::remove_var("JWT_SECRET") },
        }
    }

    #[test]
    fn validate_requires_codepool_when_run_web_and_letta_runtime() {
        let mut web_on = Config::test_stub();
        web_on.run_web = true;
        web_on.agent_runtime_mode = crate::config::AgentRuntimeMode::Letta;
        web_on.codepool_base_url = String::new();
        assert!(
            validate_runtime_config(&web_on).is_err(),
            "RUN_WEB=true with letta runtime requires CODEPOOL_BASE_URL"
        );
        web_on.codepool_base_url = "http://localhost:3030".into();
        validate_runtime_config(&web_on).expect("RUN_WEB with Codepool should pass");
    }

    #[test]
    fn validate_allows_missing_codepool_when_native_runtime() {
        let mut web_on = Config::test_stub();
        web_on.run_web = true;
        web_on.agent_runtime_mode = crate::config::AgentRuntimeMode::Native;
        web_on.codepool_base_url = String::new();
        web_on.llm_api_url = "http://bears-bifrost:8080/v1".into();
        validate_runtime_config(&web_on).expect("native runtime does not require Codepool");
    }

    #[test]
    fn standalone_web_allows_missing_codepool() {
        let prev = std::env::var("DEN_ALLOW_STANDALONE_WEB").ok();
        unsafe {
            std::env::set_var("DEN_ALLOW_STANDALONE_WEB", "true");
        }

        let mut web_on = Config::test_stub();
        web_on.run_web = true;
        web_on.codepool_base_url = String::new();
        validate_runtime_config(&web_on).expect("standalone web skips Codepool requirement");

        match prev {
            Some(v) => unsafe { std::env::set_var("DEN_ALLOW_STANDALONE_WEB", v) },
            None => unsafe { std::env::remove_var("DEN_ALLOW_STANDALONE_WEB") },
        }
    }
}
