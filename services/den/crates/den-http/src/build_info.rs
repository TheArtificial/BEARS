//! Build metadata exposed for `GET /version` on web and API.
//!
//! Most fields come from compile-time `build.rs` output, but `git_sha` and
//! `built_at_utc` can be overridden at runtime so deploy platforms can report
//! deploy-time metadata without invalidating Docker build cache layers.

use axum::{response::IntoResponse, Json};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct VersionBody {
    pub service: String,
    pub version: String,
    /// RFC 3339 UTC timestamp from runtime override (`DEN_BUILT_AT_OVERRIDE`) when present,
    /// otherwise from when `build.rs` last ran (`SOURCE_DATE_EPOCH` overrides for reproducible builds).
    pub built_at_utc: String,
    /// Git commit from runtime override (`DEN_GIT_SHA_OVERRIDE`, `GIT_SHA`, or `SOURCE_COMMIT`) when present,
    /// otherwise the compile-time `GIT_SHA` build-arg baked by `build.rs`.
    pub git_sha: String,
}

fn env_override(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

pub fn snapshot() -> VersionBody {
    VersionBody {
        service: "den".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        built_at_utc: env_override(&["DEN_BUILT_AT_OVERRIDE"])
            .unwrap_or_else(|| env!("DEN_BUILT_AT_UTC").to_string()),
        git_sha: env_override(&["DEN_GIT_SHA_OVERRIDE", "GIT_SHA", "SOURCE_COMMIT"])
            .unwrap_or_else(|| env!("DEN_GIT_SHA").to_string()),
    }
}

pub async fn json_handler() -> impl IntoResponse {
    Json(snapshot())
}
