use std::env;
use std::time::Instant;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// UTC time when the build script ran, RFC 3339 (e.g. `2026-04-21T12:34:56Z`).
/// If `SOURCE_DATE_EPOCH` is set (seconds since Unix epoch), use that for reproducible builds.
fn build_time_utc_rfc3339() -> String {
    if let Ok(epoch) = env::var("SOURCE_DATE_EPOCH") {
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build_start = Instant::now();
    println!("cargo:warning=Build script starting...");

    let built_at = build_time_utc_rfc3339();
    println!("cargo:rustc-env=DEN_BUILT_AT_UTC={}", built_at);

    let git_sha = env::var("GIT_SHA")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=DEN_GIT_SHA={}", git_sha);
    println!("cargo:rerun-if-env-changed=GIT_SHA");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    // All template groups now live in the edge crates (web -> den-web, email ->
    // den-http, api -> den-api), each embedded by its own build.rs in production.
    // The thin binary embeds nothing.

    println!(
        "cargo:warning=Build script completed in {:.2}s",
        build_start.elapsed().as_secs_f64()
    );
    Ok(())
}
