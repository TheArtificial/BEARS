//! Build script for `den-http`.
//!
//! Emits the build metadata env vars consumed by `build_info` (`DEN_BUILT_AT_UTC`,
//! `DEN_GIT_SHA`) and, in production builds (the `production` feature), embeds the
//! email template group so `email::template_environment` can `load_templates!(.., "email")`
//! without disk access. In dev builds the embed is a no-op (templates path-loaded).
use std::{fs, path::Path};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

fn emit_rerun_if_changed(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());

    if path.is_dir() {
        let mut entries = fs::read_dir(path)
            .unwrap_or_else(|err| panic!("failed to read template path {}: {err}", path.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|err| {
                panic!(
                    "failed to enumerate template path {}: {err}",
                    path.display()
                )
            });
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            emit_rerun_if_changed(&entry.path());
        }
    }
}

/// UTC time when the build script ran, RFC 3339. `SOURCE_DATE_EPOCH` (seconds since
/// the Unix epoch) overrides for reproducible builds.
fn build_time_utc_rfc3339() -> String {
    if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH") {
        if let Ok(secs) = epoch.trim().parse::<i64>() {
            if let Ok(dt) = OffsetDateTime::from_unix_timestamp(secs) {
                return dt.format(&Rfc3339).expect("RFC3339 format");
            }
        }
    }
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("RFC3339 format")
}

fn main() {
    println!(
        "cargo:rustc-env=DEN_BUILT_AT_UTC={}",
        build_time_utc_rfc3339()
    );
    let git_sha = std::env::var("GIT_SHA")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=DEN_GIT_SHA={git_sha}");
    println!("cargo:rerun-if-env-changed=GIT_SHA");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    let production_enabled = std::env::var_os("CARGO_FEATURE_PRODUCTION").is_some();
    if production_enabled {
        let email_templates_dir = std::env::var("EMAIL_TEMPLATES_DIR")
            .unwrap_or_else(|_| "src/email/templates".to_string());
        emit_rerun_if_changed(Path::new(&email_templates_dir));
        minijinja_embed::embed_templates!(&email_templates_dir, &[][..], "email");
    }
}
