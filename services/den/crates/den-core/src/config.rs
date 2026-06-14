#![allow(dead_code)]

use std::collections::HashSet;

use url::Url;

/// Human-facing product name (browser title, emails, PWA). Override with `APP_DISPLAY_NAME`.
const DEFAULT_APP_DISPLAY_NAME: &str = "BEARS";

/// Machine slug (manifest `id`, short name). Override with `APP_SLUG`.
const DEFAULT_APP_SLUG: &str = "bears";

/// Default public web origin when `WEB_SERVER_URL` is unset in **production** builds.
/// Forks should set `WEB_SERVER_URL` / `API_SERVER_URL` or change these constants.
const DEFAULT_PROD_WEB_ORIGIN: &str = "https://bears.artificial.design";
const DEFAULT_PROD_API_ORIGIN: &str = "https://api.bears.artificial.design";

/// Bifrost OpenAI-compatible API when `LLM_API_URL` is unset — matches Docker Compose `bears-bifrost:8080/v1`.
pub const DEFAULT_LLM_API_URL: &str = "http://bears-bifrost:8080/v1";

pub fn session_cookie_secure_from_env(default: bool) -> bool {
    std::env::var("SESSION_COOKIE_SECURE")
        .map(|v| match v.trim().to_ascii_lowercase().as_str() {
            "0" | "false" | "no" | "off" => false,
            "1" | "true" | "yes" | "on" => true,
            _ => default,
        })
        .unwrap_or(default)
}

/// Explicit fixture profiles for feature-gated web UI smoke testing.
///
/// These profiles are only usable when the `web-ui-fixtures` Cargo feature is compiled in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiFixtureProfile {
    BearDetailsRich,
    BearDetailsWarning,
    BearChatBasic,
    BearChatError,
}

impl UiFixtureProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BearDetailsRich => "bear-details-rich",
            Self::BearDetailsWarning => "bear-details-warning",
            Self::BearChatBasic => "bear-chat-basic",
            Self::BearChatError => "bear-chat-error",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "bear-details-rich" => Some(Self::BearDetailsRich),
            "bear-details-warning" => Some(Self::BearDetailsWarning),
            "bear-chat-basic" => Some(Self::BearChatBasic),
            "bear-chat-error" => Some(Self::BearChatError),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub templates_dir: String,
    pub database_url: String,

    pub mailgun_api_key: String,
    pub mailgun_domain: String,

    /// Shown as the sender display name in outbound mail.
    pub app_display_name: String,
    /// Local-part@domain for Mailgun From (full address). Override with `MAIL_FROM_ADDRESS`.
    pub mail_from_address: String,

    pub telemetry_url_prefix: String,
    pub email_verify_url_prefix: String,

    /// Short machine id (`manifest.json` id, etc.).
    pub app_slug: String,

    /// Optional cookie `Domain` attribute (production). When unset or empty, cookies are host-only.
    pub session_cookie_domain: Option<String>,

    /// Enable the web server (`RUN_WEB`, default false).
    pub run_web: bool,

    /// Enable the API server (`RUN_API`, default false).
    pub run_api: bool,

    /// Enable background workers (`RUN_WORKERS`).
    ///
    /// Current worker set includes the memory-curate queue runner used to process
    /// queued `memory_curate` reflection runs.
    pub run_workers: bool,

    /// Enable the API-only ACP gateway (`ACP_GATEWAY_ENABLED`).
    pub acp_gateway_enabled: bool,

    pub web_port: u16,
    pub api_port: u16,

    /// Public base URL for the **web** service (no trailing slash). Links, redirects, CORS.
    pub web_server_url: String,
    /// Public base URL for the **API** service (no trailing slash).
    pub api_server_url: String,

    /// Shared secret for internal API endpoints (`DEN_INTERNAL_TOKEN`). Empty = internal auth disabled.
    pub den_internal_token: String,

    /// Bifrost gateway base URL (no trailing slash), e.g. `http://bears-bifrost:8080`. Empty = skip HTTP check.
    pub bifrost_base_url: String,
    /// BEARS model metadata URL served by the Bifrost image, e.g. `http://bears-bifrost:8081/bears/models`.
    /// Empty = derive from `bifrost_base_url` where possible or skip Bifrost metadata.
    pub bifrost_metadata_url: String,

    /// OpenAI-compatible inference API (no trailing slash), e.g. `http://bears-bifrost:8080/v1`.
    /// Used by the Den-native agent loop. Defaults from `LLM_API_URL`, then `BIFROST_BASE_URL/v1`.
    pub llm_api_url: String,
    /// Optional bearer token for `llm_api_url` (`LLM_API_KEY` or `OPENAI_API_KEY`).
    pub llm_api_key: String,
    /// Default model handle for native runtime turns (`DEFAULT_LLM_MODEL`).
    pub default_llm_model: String,
    /// Directory for per-Bear SQLite databases (`BEAR_SQLITE_DATA_DIR`, default `/var/lib/den/bear-sqlite`).
    pub bear_sqlite_data_dir: String,

    /// S3-compatible endpoint (e.g. `http://bears-garage:3900`). Empty = media upload disabled.
    pub s3_endpoint: String,
    /// S3 bucket for chat media and generated images.
    pub s3_bucket: String,
    /// S3 region (Garage default: `garage`).
    pub s3_region: String,
    /// S3 access key id.
    pub s3_access_key_id: String,
    /// S3 secret access key.
    pub s3_secret_access_key: String,
    /// Public URL prefix for presigned download URLs (the origin browsers can reach).
    /// Falls back to `s3_endpoint` when empty.
    pub s3_public_url: String,
    /// Use path-style addressing (`endpoint/bucket/key` instead of `bucket.endpoint/key`).
    /// Required for Garage and most self-hosted S3; defaults to true.
    pub s3_force_path_style: bool,

    /// Maximum number of connections in the SQLx pool (`DB_MAX_CONNECTIONS`, default 5).
    pub db_max_connections: u32,
    /// Seconds to wait for a connection from the pool before timing out (`DB_ACQUIRE_TIMEOUT_SECS`, default 3).
    pub db_acquire_timeout_secs: u64,
    /// Seconds a connection can sit idle before being closed (`DB_IDLE_TIMEOUT_SECS`, default 600).
    /// Set to 0 to disable idle reaping.
    pub db_idle_timeout_secs: u64,

    /// PAT with `read:packages` for `web::status` GHCR comparison (optional; when empty, registry columns show "not configured").
    pub github_packages_token: String,
    /// GitHub org or username that owns `den` / `codepool` images on GHCR (e.g. `theartificial`).
    pub ghcr_packages_owner: String,
    /// `org` or `user` — used with GitHub Packages REST paths.
    pub ghcr_packages_owner_kind: String,

    /// Search provider for Den web search tools (`DEN_SEARCH_PROVIDER`, e.g. `brave`). Empty disables search.
    pub den_search_provider: String,
    /// Brave Search API key (`BRAVE_SEARCH_API_KEY`) when `DEN_SEARCH_PROVIDER=brave`.
    pub brave_search_api_key: String,
    /// Max results returned by Den search tools (`DEN_SEARCH_MAX_RESULTS`, default 5, clamped 1..10).
    pub den_search_max_results: usize,

    /// Derived recall vector store (Qdrant) base URL, e.g. `http://bears-qdrant:6333`
    /// (`QDRANT_URL`). `None`/empty disables recall — Den falls back to keyword search (ADR-0038).
    pub qdrant_url: Option<String>,
    /// Active platform embedding standard id (`EMBEDDING_STANDARD`, default `bears-embed-v1`).
    pub embedding_standard: String,
    /// Embedding model for the active standard (`EMBEDDING_MODEL`, default `text-embedding-3-small`).
    pub embedding_model: String,
    /// Embedding vector dimensions for the active standard (`EMBEDDING_DIMENSIONS`, default 1536).
    pub embedding_dimensions: u32,

    /// Explicit web UI fixture profile for browser smoke testing.
    ///
    /// This is a runtime selector, not a generic development mode switch. The named profile is
    /// only honored when the binary is compiled with the `web-ui-fixtures` Cargo feature.
    pub ui_fixture_profile: Option<UiFixtureProfile>,
}

impl Config {
    /// Web origin without trailing slash — use for path suffixes: `{}{path}`.
    pub fn web_public_origin(&self) -> String {
        self.web_server_url.trim_end_matches('/').to_string()
    }

    pub fn cors_allowed_origins(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for raw in [&self.web_server_url, &self.api_server_url] {
            match Url::parse(raw.as_str()) {
                Ok(u) => {
                    let origin = u.origin().ascii_serialization();
                    if origin != "null" && seen.insert(origin.clone()) {
                        out.push(origin);
                    }
                }
                Err(e) => {
                    tracing::warn!("Could not parse URL for CORS origin (value={raw:?}): {e}")
                }
            }
        }
        out
    }

    /// Load configuration from process environment (and optional `.env` via dotenvy).
    ///
    /// Call once at process startup; thread an [`std::sync::Arc`] through services that need it.
    pub fn load() -> Self {
        if dotenvy::dotenv().is_ok() {
            tracing::info!("Loaded .env file");
        }

        let app_display_name = std::env::var("APP_DISPLAY_NAME")
            .unwrap_or_else(|_| DEFAULT_APP_DISPLAY_NAME.to_string());
        let app_slug = std::env::var("APP_SLUG").unwrap_or_else(|_| DEFAULT_APP_SLUG.to_string());

        fn parse_bool_env(name: &str, default: bool) -> bool {
            std::env::var(name)
                .unwrap_or_else(|_| {
                    if default {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                })
                .parse::<bool>()
                .unwrap_or_else(|_| {
                    tracing::warn!(
                        "Invalid {} environment variable. Expected 'true' or 'false'. Defaulting to {}.",
                        name,
                        default
                    );
                    default
                })
        }

        let (run_web, run_api, run_workers) = if let Ok(server_mode) = std::env::var("SERVER_MODE")
        {
            match server_mode.to_lowercase().as_str() {
                "web" => (true, false, true),
                "api" => (false, true, true),
                "both" => (true, true, true),
                _ => {
                    tracing::warn!(
                        "Invalid SERVER_MODE value '{}'. Use RUN_WEB, RUN_API, RUN_WORKERS instead.",
                        server_mode
                    );
                    (false, false, false)
                }
            }
        } else {
            (
                parse_bool_env("RUN_WEB", false),
                parse_bool_env("RUN_API", false),
                parse_bool_env("RUN_WORKERS", false),
            )
        };

        let acp_gateway_enabled = parse_bool_env("ACP_GATEWAY_ENABLED", false);

        let ui_fixture_profile = match std::env::var("UI_FIXTURE_PROFILE") {
            Ok(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    match UiFixtureProfile::parse(trimmed) {
                        Some(profile) => Some(profile),
                        None => {
                            tracing::warn!(
                                "Invalid UI_FIXTURE_PROFILE value '{}'. Supported values: bear-details-rich, bear-details-warning, bear-chat-basic, bear-chat-error.",
                                trimmed
                            );
                            None
                        }
                    }
                }
            }
            Err(_) => None,
        };

        let web_port = std::env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse::<u16>()
            .unwrap_or_else(|_| {
                tracing::warn!("Invalid PORT environment variable. Defaulting to 3000");
                3000
            });

        let api_port = std::env::var("API_PORT")
            .unwrap_or_else(|_| "3001".to_string())
            .parse::<u16>()
            .unwrap_or_else(|_| {
                tracing::warn!("Invalid API_PORT environment variable. Defaulting to 3001");
                3001
            });

        let web_server_url = std::env::var("WEB_SERVER_URL").unwrap_or_else(|_| {
            #[cfg(feature = "production")]
            {
                DEFAULT_PROD_WEB_ORIGIN.to_string()
            }
            #[cfg(not(feature = "production"))]
            {
                format!("http://localhost:{web_port}")
            }
        });
        let web_server_url = trim_url_for_storage(web_server_url);

        let api_server_url = std::env::var("API_SERVER_URL").unwrap_or_else(|_| {
            #[cfg(feature = "production")]
            {
                DEFAULT_PROD_API_ORIGIN.to_string()
            }
            #[cfg(not(feature = "production"))]
            {
                format!("http://localhost:{api_port}")
            }
        });
        let api_server_url = trim_url_for_storage(api_server_url);

        let public_web_base = format_url_prefix(&web_server_url);
        let email_verify_url_prefix = format!("{}settings/email/verify/", public_web_base);
        let telemetry_url_prefix = format!("{}telemetry/", public_web_base);

        let session_cookie_domain = std::env::var("SESSION_COOKIE_DOMAIN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let mail_from_address = std::env::var("MAIL_FROM_ADDRESS").unwrap_or_else(|_| {
            derive_default_mail_from(&web_server_url)
                .unwrap_or_else(|| "noreply@bears.artificial.design".to_string())
        });

        let den_internal_token = std::env::var("DEN_INTERNAL_TOKEN").unwrap_or_default();

        let bifrost_base_url = std::env::var("BIFROST_BASE_URL").unwrap_or_default();
        let bifrost_base_url = bifrost_base_url.trim_end_matches('/').to_string();
        let bifrost_metadata_url = std::env::var("BIFROST_METADATA_URL")
            .unwrap_or_else(|_| {
                if bifrost_base_url.is_empty() {
                    String::new()
                } else {
                    bifrost_base_url.replace(":8080", ":8081") + "/bears/models"
                }
            })
            .trim()
            .to_string();

        let llm_api_url = std::env::var("LLM_API_URL")
            .ok()
            .map(|v| v.trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                if bifrost_base_url.is_empty() {
                    String::new()
                } else {
                    format!("{bifrost_base_url}/v1")
                }
            });
        let llm_api_key = std::env::var("LLM_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .unwrap_or_default();
        let default_llm_model =
            std::env::var("DEFAULT_LLM_MODEL").unwrap_or_else(|_| "openai/gpt-4.1".to_string());
        let bear_sqlite_data_dir = std::env::var("BEAR_SQLITE_DATA_DIR")
            .unwrap_or_else(|_| "/var/lib/den/bear-sqlite".to_string());

        let s3_endpoint = std::env::var("S3_ENDPOINT")
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string();
        let s3_bucket = std::env::var("S3_BUCKET").unwrap_or_default();
        let s3_region = std::env::var("S3_REGION").unwrap_or_else(|_| "garage".to_string());
        let s3_access_key_id = std::env::var("S3_ACCESS_KEY_ID").unwrap_or_default();
        let s3_secret_access_key = std::env::var("S3_SECRET_ACCESS_KEY").unwrap_or_default();
        let s3_public_url = std::env::var("S3_PUBLIC_URL")
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string();
        let s3_force_path_style = parse_bool_env("S3_FORCE_PATH_STYLE", true);

        let db_max_connections: u32 = std::env::var("DB_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "5".to_string())
            .parse()
            .unwrap_or_else(|_| {
                tracing::warn!("Invalid DB_MAX_CONNECTIONS, defaulting to 5");
                5
            });
        let db_acquire_timeout_secs: u64 = std::env::var("DB_ACQUIRE_TIMEOUT_SECS")
            .unwrap_or_else(|_| "3".to_string())
            .parse()
            .unwrap_or_else(|_| {
                tracing::warn!("Invalid DB_ACQUIRE_TIMEOUT_SECS, defaulting to 3");
                3
            });
        let db_idle_timeout_secs: u64 = std::env::var("DB_IDLE_TIMEOUT_SECS")
            .unwrap_or_else(|_| "600".to_string())
            .parse()
            .unwrap_or_else(|_| {
                tracing::warn!("Invalid DB_IDLE_TIMEOUT_SECS, defaulting to 600");
                600
            });

        let github_packages_token = std::env::var("GITHUB_PACKAGES_TOKEN").unwrap_or_default();
        let ghcr_packages_owner = std::env::var("GHCR_PACKAGES_OWNER")
            .unwrap_or_default()
            .trim()
            .to_string();
        let ghcr_packages_owner_kind = std::env::var("GHCR_PACKAGES_OWNER_KIND")
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let ghcr_packages_owner_kind = if ghcr_packages_owner_kind.is_empty()
            || matches!(ghcr_packages_owner_kind.as_str(), "org" | "user")
        {
            ghcr_packages_owner_kind
        } else {
            tracing::warn!("Invalid GHCR_PACKAGES_OWNER_KIND (expected org|user). Leaving empty.");
            String::new()
        };

        let den_search_provider = std::env::var("DEN_SEARCH_PROVIDER")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let brave_search_api_key = std::env::var("BRAVE_SEARCH_API_KEY").unwrap_or_default();
        let den_search_max_results = std::env::var("DEN_SEARCH_MAX_RESULTS")
            .unwrap_or_else(|_| "5".to_string())
            .parse::<usize>()
            .unwrap_or_else(|_| {
                tracing::warn!("Invalid DEN_SEARCH_MAX_RESULTS, defaulting to 5");
                5
            })
            .clamp(1, 10);

        let qdrant_url = std::env::var("QDRANT_URL")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty());
        let embedding_standard = std::env::var("EMBEDDING_STANDARD")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "bears-embed-v1".to_string());
        let embedding_model = std::env::var("EMBEDDING_MODEL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "text-embedding-3-small".to_string());
        let embedding_dimensions: u32 = std::env::var("EMBEDDING_DIMENSIONS")
            .unwrap_or_else(|_| "1536".to_string())
            .parse()
            .unwrap_or_else(|_| {
                tracing::warn!("Invalid EMBEDDING_DIMENSIONS, defaulting to 1536");
                1536
            });

        Config {
            templates_dir: std::env::var("TEMPLATES_DIR")
                .unwrap_or("crates/den-web/src/templates".to_string()),
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL"),

            mailgun_api_key: std::env::var("MAILGUN_API_KEY").unwrap_or_default(),
            mailgun_domain: std::env::var("MAILGUN_DOMAIN").unwrap_or_default(),

            app_display_name,
            mail_from_address,

            telemetry_url_prefix,
            email_verify_url_prefix,

            app_slug,

            session_cookie_domain,

            run_web,
            run_api,
            run_workers,
            acp_gateway_enabled,
            web_port,
            api_port,
            web_server_url,
            api_server_url,
            den_internal_token,
            bifrost_base_url,
            bifrost_metadata_url,
            llm_api_url,
            llm_api_key,
            default_llm_model,
            bear_sqlite_data_dir,
            s3_endpoint,
            s3_bucket,
            s3_region,
            s3_access_key_id,
            s3_secret_access_key,
            s3_public_url,
            s3_force_path_style,
            db_max_connections,
            db_acquire_timeout_secs,
            db_idle_timeout_secs,
            github_packages_token,
            ghcr_packages_owner,
            ghcr_packages_owner_kind,
            den_search_provider,
            brave_search_api_key,
            den_search_max_results,
            qdrant_url,
            embedding_standard,
            embedding_model,
            embedding_dimensions,
            ui_fixture_profile,
        }
    }
}

fn trim_url_for_storage(url: String) -> String {
    url.trim_end_matches('/').to_string()
}

/// Ensures a trailing slash for URL-prefix style concatenation.
fn format_url_prefix(base: &str) -> String {
    format!("{}/", base.trim_end_matches('/'))
}

/// If `WEB_SERVER_URL` is a normal `https` host, use `noreply@<host>`.
fn derive_default_mail_from(web_server_url: &str) -> Option<String> {
    let u = Url::parse(web_server_url).ok()?;
    let host = u.host_str()?;
    if host == "localhost" || host.starts_with('[') {
        return None;
    }
    Some(format!("noreply@{host}"))
}

#[cfg(any(test, feature = "test-util"))]
impl Config {
    /// Minimal config for unit tests that only need URL / branding fields.
    pub fn test_stub() -> Self {
        Self {
            templates_dir: "x".into(),
            database_url: "postgres://localhost/den_test".into(),
            mailgun_api_key: String::new(),
            mailgun_domain: String::new(),
            app_display_name: "Test".into(),
            mail_from_address: "noreply@localhost".into(),
            telemetry_url_prefix: "http://localhost:3000/telemetry/".into(),
            email_verify_url_prefix: "http://localhost:3000/settings/email/verify/".into(),
            app_slug: "test".into(),
            session_cookie_domain: None,
            run_web: false,
            run_api: false,
            run_workers: false,
            acp_gateway_enabled: false,
            web_port: 3000,
            api_port: 3001,
            web_server_url: "http://localhost:3000".into(),
            api_server_url: "http://localhost:3001".into(),
            den_internal_token: String::new(),
            bifrost_base_url: String::new(),
            bifrost_metadata_url: String::new(),
            llm_api_url: String::new(),
            llm_api_key: String::new(),
            default_llm_model: "gpt-4.1".into(),
            bear_sqlite_data_dir: "/var/lib/den/bear-sqlite".into(),
            s3_endpoint: String::new(),
            s3_bucket: String::new(),
            s3_region: "garage".into(),
            s3_access_key_id: String::new(),
            s3_secret_access_key: String::new(),
            s3_public_url: String::new(),
            s3_force_path_style: true,
            db_max_connections: 5,
            db_acquire_timeout_secs: 3,
            db_idle_timeout_secs: 600,
            github_packages_token: String::new(),
            ghcr_packages_owner: String::new(),
            ghcr_packages_owner_kind: String::new(),
            den_search_provider: String::new(),
            brave_search_api_key: String::new(),
            den_search_max_results: 5,
            qdrant_url: None,
            embedding_standard: "bears-embed-v1".into(),
            embedding_model: "text-embedding-3-small".into(),
            embedding_dimensions: 1536,
            ui_fixture_profile: None,
        }
    }
}

/// Whether startup must require a non-empty `JWT_SECRET`.
///
/// Production builds always require it; dev builds require it only when the API
/// service runs (it mints/validates bearer tokens). Shared by the binary's
/// startup validation and the web edge's stack-health check.
#[must_use]
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
