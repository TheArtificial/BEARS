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

    // Do not bake deploy commit metadata into the binary. Docker/Coolify often
    // changes commit build args for non-Den changes, which invalidates the cargo
    // build layer and effectively forces a full Rust rebuild. Runtime surfaces use
    // DEN_GIT_SHA_OVERRIDE / SOURCE_COMMIT instead (see den-http::build_info).
    println!("cargo:rustc-env=DEN_GIT_SHA=unknown");
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
