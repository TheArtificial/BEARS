use std::{path::Path, process::Command};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

fn main() {
    for name in [
        "GITHUB_SHA",
        "BEAR_ARMATURE_BUILD_SHA",
        "DEN_ACP_ADAPTER_BUILD_SHA",
        "BEARS_ACP_ADAPTER_BUILD_SHA",
        "SOURCE_DATE_EPOCH",
        "BEAR_ARMATURE_RELEASE_VERSION",
        "DEN_ACP_ADAPTER_RELEASE_VERSION",
        "BEARS_ACP_ADAPTER_RELEASE_VERSION",
        "BEAR_ARMATURE_MACOS_INSTALLER_IDENTITY",
        "DEN_ACP_ADAPTER_MACOS_INSTALLER_IDENTITY",
        "BEARS_ACP_ADAPTER_MACOS_INSTALLER_IDENTITY",
        "BEAR_ARMATURE_MACOS_INSTALLER_TEAM_ID",
        "DEN_ACP_ADAPTER_MACOS_INSTALLER_TEAM_ID",
        "BEARS_ACP_ADAPTER_MACOS_INSTALLER_TEAM_ID",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/main.rs");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");

    let built_at = build_time_utc_rfc3339();
    println!("cargo:rustc-env=DEN_ACP_ADAPTER_BUILT_AT_UTC={built_at}");

    let adapter_version = env_with_aliases(
        &[
            "BEAR_ARMATURE_RELEASE_VERSION",
            "DEN_ACP_ADAPTER_RELEASE_VERSION",
            "BEARS_ACP_ADAPTER_RELEASE_VERSION",
        ],
    )
    .unwrap_or_else(|| {
        std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION set by Cargo")
    });
    println!("cargo:rustc-env=DEN_ACP_ADAPTER_VERSION={adapter_version}");

    let build_sha = env_with_aliases(&[
        "BEAR_ARMATURE_BUILD_SHA",
        "DEN_ACP_ADAPTER_BUILD_SHA",
        "BEARS_ACP_ADAPTER_BUILD_SHA",
    ])
    .or_else(|| {
        std::env::var("GITHUB_SHA")
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
    .or_else(local_repo_head_sha)
    .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=DEN_ACP_ADAPTER_GIT_SHA={build_sha}");

    forward_optional_env_aliases(
        &[
            "BEAR_ARMATURE_MACOS_INSTALLER_IDENTITY",
            "DEN_ACP_ADAPTER_MACOS_INSTALLER_IDENTITY",
            "BEARS_ACP_ADAPTER_MACOS_INSTALLER_IDENTITY",
        ],
        "BEARS_ACP_ADAPTER_MACOS_INSTALLER_IDENTITY",
    );
    forward_optional_env_aliases(
        &[
            "BEAR_ARMATURE_MACOS_INSTALLER_TEAM_ID",
            "DEN_ACP_ADAPTER_MACOS_INSTALLER_TEAM_ID",
            "BEARS_ACP_ADAPTER_MACOS_INSTALLER_TEAM_ID",
        ],
        "BEARS_ACP_ADAPTER_MACOS_INSTALLER_TEAM_ID",
    );
}

fn env_with_aliases(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
}

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

fn local_repo_head_sha() -> Option<String> {
    let repo_root = Path::new("../..");
    if !repo_root.join(".git").exists() || !repo_root.join("tools/bear-armature").is_dir() {
        return None;
    }

    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            let top = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let adapter_path = Path::new(&top).join("tools/bear-armature");
            if adapter_path.is_dir() {
                Some(())
            } else {
                None
            }
        })?;

    Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn forward_optional_env_aliases(names: &[&str], rustc_env_name: &str) {
    if let Some(value) = env_with_aliases(names) {
        // update.rs reads the legacy BEARS_* names via option_env!.
        println!("cargo:rustc-env={rustc_env_name}={value}");
    }
}
