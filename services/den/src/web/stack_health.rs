//! Runtime probes and config checks for the public **`/status`** page ([`crate::web::status`]).
//!
//! Combines **runtime probes** (PostgreSQL, upstream HTTP) with **low-cost config sanity**
//! aligned with the repo’s **`services/preflight`** script: JWT when
//! required, `LETTA_PG_URI` / `LLM_API_URL` shape, and `OPENAI_API_KEY` presence warnings.

use std::time::Duration;

use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::time::timeout;
use url::Url;

use crate::startup;
use crate::web::AppState;

/// Wall-clock timeout for each upstream HTTP health call (Letta, Codepool, Bifrost).
const HTTP_PROBE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Serialize)]
pub struct StackHealthReport {
    /// `true` when no check has [`CheckState::Fail`]. Warnings still yield `true`.
    pub ok: bool,
    pub checked_at: String,
    pub checks: Vec<HealthCheck>,
}

#[derive(Clone, Serialize)]
pub struct HealthCheck {
    pub id: &'static str,
    pub label: &'static str,
    pub state: CheckState,
    pub detail: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckState {
    Ok,
    Warn,
    Fail,
    Skipped,
}

impl StackHealthReport {
    fn from_checks(checks: Vec<HealthCheck>) -> Self {
        let ok = checks.iter().all(|c| c.state != CheckState::Fail);
        let checked_at = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".to_string());
        Self {
            ok,
            checked_at,
            checks,
        }
    }
}

/// Row shape for the `/status` HTML table.
#[derive(Serialize)]
pub struct StackHealthTemplateRow {
    pub id: &'static str,
    pub label: &'static str,
    pub state: &'static str,
    pub detail: String,
}

pub async fn gather(state: &AppState) -> StackHealthReport {
    let cfg = state.config.as_ref();

    let mut checks: Vec<HealthCheck> = Vec::new();

    checks.push(jwt_check(cfg));
    checks.push(den_database_url_shape(cfg));
    if let Some(c) = llm_api_url_shape() {
        checks.push(c);
    }
    checks.push(openai_key_warn());
    checks.push(web_server_url_shape(cfg));

    let den_pg = check_den_postgres(state.sqlx_pool()).await;
    let bifrost_h =
        check_bifrost_http(&cfg.bifrost_base_url, &cfg.bifrost_metadata_url).await;

    checks.push(den_pg);
    checks.push(bifrost_h);

    StackHealthReport::from_checks(checks)
}

fn jwt_check(cfg: &crate::config::Config) -> HealthCheck {
    if !startup::requires_jwt_secret(cfg) {
        return HealthCheck {
            id: "jwt_secret",
            label: "JWT_SECRET",
            state: CheckState::Skipped,
            detail: "not required for this build/run mode".into(),
        };
    }
    let secret = std::env::var("JWT_SECRET").unwrap_or_default();
    if secret.trim().is_empty() {
        HealthCheck {
            id: "jwt_secret",
            label: "JWT_SECRET",
            state: CheckState::Fail,
            detail:
                "empty but required (production build or RUN_API); preflight also requires this"
                    .into(),
        }
    } else {
        HealthCheck {
            id: "jwt_secret",
            label: "JWT_SECRET",
            state: CheckState::Ok,
            detail: "set".into(),
        }
    }
}

fn den_database_url_shape(cfg: &crate::config::Config) -> HealthCheck {
    match Url::parse(cfg.database_url.trim()) {
        Ok(u) => {
            let scheme = u.scheme();
            if matches!(scheme, "postgres" | "postgresql") {
                if u.host_str().is_none() {
                    HealthCheck {
                        id: "database_url_shape",
                        label: "DATABASE_URL (shape)",
                        state: CheckState::Warn,
                        detail: "missing host (preflight requires a hostname)".into(),
                    }
                } else {
                    HealthCheck {
                        id: "database_url_shape",
                        label: "DATABASE_URL (shape)",
                        state: CheckState::Ok,
                        detail: "PostgreSQL URI with host".into(),
                    }
                }
            } else {
                HealthCheck {
                    id: "database_url_shape",
                    label: "DATABASE_URL (shape)",
                    state: CheckState::Warn,
                    detail: format!("expected postgres:// or postgresql://, got scheme {scheme:?}"),
                }
            }
        }
        Err(e) => HealthCheck {
            id: "database_url_shape",
            label: "DATABASE_URL (shape)",
            state: CheckState::Warn,
            detail: format!("parse error: {e}"),
        },
    }
}

fn llm_api_url_shape() -> Option<HealthCheck> {
    let raw = std::env::var("LLM_API_URL").ok()?;
    let v = raw.trim();
    if v.is_empty() {
        return None;
    }
    Some(match Url::parse(v) {
        Ok(u) => {
            let ok = (u.scheme() == "http" || u.scheme() == "https") && u.host_str().is_some();
            if ok {
                HealthCheck {
                    id: "llm_api_url_shape",
                    label: "LLM_API_URL (shape)",
                    state: CheckState::Ok,
                    detail: "valid http(s) URL (Letta → Bifrost; mirrors preflight)".into(),
                }
            } else {
                HealthCheck {
                    id: "llm_api_url_shape",
                    label: "LLM_API_URL (shape)",
                    state: CheckState::Warn,
                    detail: "must be http(s) with a host (see preflight)".into(),
                }
            }
        }
        Err(e) => HealthCheck {
            id: "llm_api_url_shape",
            label: "LLM_API_URL (shape)",
            state: CheckState::Warn,
            detail: format!("parse error: {e}"),
        },
    })
}

fn openai_key_warn() -> HealthCheck {
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    if key.trim().is_empty() {
        HealthCheck {
            id: "openai_api_key",
            label: "OPENAI_API_KEY",
            state: CheckState::Warn,
            detail:
                "empty — embeddings and direct OpenAI calls may fail (preflight warns similarly)"
                    .into(),
        }
    } else {
        HealthCheck {
            id: "openai_api_key",
            label: "OPENAI_API_KEY",
            state: CheckState::Ok,
            detail: "set".into(),
        }
    }
}

fn web_server_url_shape(cfg: &crate::config::Config) -> HealthCheck {
    match Url::parse(cfg.web_server_url.trim()) {
        Ok(u) => {
            let ok = (u.scheme() == "http" || u.scheme() == "https") && u.host_str().is_some();
            if ok {
                HealthCheck {
                    id: "web_server_url_shape",
                    label: "WEB_SERVER_URL (shape)",
                    state: CheckState::Ok,
                    detail: "valid http(s) URL (preflight)".into(),
                }
            } else {
                HealthCheck {
                    id: "web_server_url_shape",
                    label: "WEB_SERVER_URL (shape)",
                    state: CheckState::Warn,
                    detail: "expected http(s) with a host".into(),
                }
            }
        }
        Err(e) => HealthCheck {
            id: "web_server_url_shape",
            label: "WEB_SERVER_URL (shape)",
            state: CheckState::Warn,
            detail: format!("parse error: {e}"),
        },
    }
}

async fn check_den_postgres(pool: &PgPool) -> HealthCheck {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
    {
        Ok(_) => HealthCheck {
            id: "den_postgres",
            label: "Den PostgreSQL",
            state: CheckState::Ok,
            detail: "SELECT 1 on DATABASE_URL pool succeeded".into(),
        },
        Err(e) => HealthCheck {
            id: "den_postgres",
            label: "Den PostgreSQL",
            state: CheckState::Fail,
            detail: e.to_string(),
        },
    }
}

async fn check_bifrost_http(base: &str, metadata_url: &str) -> HealthCheck {
    if base.trim().is_empty() {
        return HealthCheck {
            id: "bifrost",
            label: "Bifrost",
            state: CheckState::Skipped,
            detail:
                "BIFROST_BASE_URL unset — set e.g. http://bears-bifrost:8080 to probe the gateway"
                    .into(),
        };
    }

    let client = match reqwest::Client::builder()
        .timeout(HTTP_PROBE_TIMEOUT)
        .connect_timeout(Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return HealthCheck {
                id: "bifrost",
                label: "Bifrost",
                state: CheckState::Fail,
                detail: format!("reqwest client: {e}"),
            };
        }
    };

    let url = format!("{}/health", base.trim_end_matches('/'));
    match timeout(HTTP_PROBE_TIMEOUT, client.get(&url).send()).await {
        Err(_) => HealthCheck {
            id: "bifrost",
            label: "Bifrost",
            state: CheckState::Fail,
            detail: format!("timeout after {}s ({url})", HTTP_PROBE_TIMEOUT.as_secs()),
        },
        Ok(Err(e)) => HealthCheck {
            id: "bifrost",
            label: "Bifrost",
            state: CheckState::Fail,
            detail: e.to_string(),
        },
        Ok(Ok(resp)) => {
            let status = resp.status();
            if status.is_success() {
                let metadata_detail = check_bifrost_metadata(&client, metadata_url).await;
                HealthCheck {
                    id: "bifrost",
                    label: "Bifrost",
                    state: metadata_detail.0,
                    detail: format!("HTTP {status} from {url}; {}", metadata_detail.1),
                }
            } else {
                HealthCheck {
                    id: "bifrost",
                    label: "Bifrost",
                    state: CheckState::Fail,
                    detail: format!("HTTP {status} from {url}"),
                }
            }
        }
    }
}

async fn check_bifrost_metadata(
    client: &reqwest::Client,
    metadata_url: &str,
) -> (CheckState, String) {
    let url = metadata_url.trim();
    if url.is_empty() {
        return (
            CheckState::Warn,
            "BIFROST_METADATA_URL unset — model context windows cannot be verified".into(),
        );
    }

    match timeout(HTTP_PROBE_TIMEOUT, client.get(url).send()).await {
        Err(_) => (
            CheckState::Warn,
            format!(
                "metadata timeout after {}s ({url})",
                HTTP_PROBE_TIMEOUT.as_secs()
            ),
        ),
        Ok(Err(e)) => (CheckState::Warn, format!("metadata request failed: {e}")),
        Ok(Ok(resp)) => {
            let status = resp.status();
            if !status.is_success() {
                return (
                    CheckState::Warn,
                    format!("metadata HTTP {status} from {url}"),
                );
            }
            let text = match resp.text().await {
                Ok(t) => t,
                Err(e) => return (CheckState::Warn, format!("metadata body failed: {e}")),
            };
            let value: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => return (CheckState::Warn, format!("metadata JSON parse failed: {e}")),
            };
            let models = value
                .get("models")
                .and_then(|x| x.as_array())
                .map(|xs| {
                    xs.iter()
                        .filter(|m| m.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true))
                        .count()
                })
                .unwrap_or(0);
            if models == 0 {
                (
                    CheckState::Warn,
                    "metadata returned no enabled models".into(),
                )
            } else {
                (
                    CheckState::Ok,
                    format!("metadata OK ({models} enabled models)"),
                )
            }
        }
    }
}
