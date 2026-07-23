//! Den-hosted, work-run-scoped Rust dependency preparation.
//!
//! This module deliberately does not execute Cargo in the sandbox. The concrete
//! runner is injected by the Den composition layer and must authorize the work
//! run, resolve its checkout, and use the provider-controlled helper.

use std::path::{Component, Path};

use den_core::{
    tools::{
        arguments::{
            PrepareRustDependenciesArguments, RustDependencyPreparation, RustDependencyResolution,
        },
        context::DenToolInvocationContext,
    },
    DenError,
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareRustDependenciesRequest {
    pub work_run_id: Uuid,
    /// A checkout-relative `Cargo.toml` path. It is never an absolute host path.
    pub manifest_path: String,
    pub package: String,
    pub resolution: RustDependencyResolution,
    pub preparation: RustDependencyPreparation,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrepareRustDependenciesResult {
    /// `prepared`, `failed`, `rejected`, or `unavailable`.
    pub status: String,
    /// Stable, machine-readable reason for the result.
    pub code: String,
    /// The broker lifecycle stage that produced the result.
    pub stage: String,
    /// Whether retrying unchanged input can reasonably succeed.
    pub retryable: bool,
    pub content: String,
    pub lockfile_changed: bool,
}

impl PrepareRustDependenciesResult {
    fn rejected(code: &str, content: &str) -> Self {
        Self {
            status: "rejected".to_string(),
            code: code.to_string(),
            stage: "authorize".to_string(),
            retryable: false,
            content: content.to_string(),
            lockfile_changed: false,
        }
    }
}

#[allow(async_fn_in_trait)]
pub trait RustDependencyPreparationRunner: Send + Sync {
    /// Authorizes `work_run_id`, resolves its checkout, and invokes the fixed
    /// provider-controlled Cargo helper. Implementations must reject a manifest
    /// outside that checkout and must not accept arbitrary Cargo arguments.
    async fn prepare_rust_dependencies(
        &self,
        request: PrepareRustDependenciesRequest,
    ) -> Result<PrepareRustDependenciesResult, DenError>;
}

/// Validates model-provided data before it reaches the privileged runner.
///
/// `work_run_id` is required because a work profile alone is not authority to
/// operate on a checkout. `manifest_path` is kept checkout-relative so it cannot
/// name an arbitrary host path.
pub async fn execute_prepare_rust_dependencies(
    runner: &impl RustDependencyPreparationRunner,
    invocation: &DenToolInvocationContext,
    arguments: PrepareRustDependenciesArguments,
) -> Result<PrepareRustDependenciesResult, DenError> {
    let Some(work_run_id) = invocation.work_run_id else {
        return Ok(PrepareRustDependenciesResult::rejected(
            "missing_work_run_binding",
            "Dependency preparation requires an active work-run binding; no Cargo helper was invoked.",
        ));
    };
    validate_manifest_path(&arguments.manifest_path)?;
    validate_package_name(&arguments.package)?;

    runner
        .prepare_rust_dependencies(PrepareRustDependenciesRequest {
            work_run_id,
            manifest_path: arguments.manifest_path,
            package: arguments.package,
            resolution: arguments.resolution,
            preparation: arguments.preparation,
        })
        .await
}

fn validate_manifest_path(path: &str) -> Result<(), DenError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.file_name().is_none_or(|name| name != "Cargo.toml")
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(DenError::ValidationError(
            "manifest_path must be a checkout-relative path ending in Cargo.toml".to_string(),
        ));
    }
    Ok(())
}

fn validate_package_name(package: &str) -> Result<(), DenError> {
    if package.is_empty()
        || package.len() > 128
        || !package
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(DenError::ValidationError(
            "package must be a non-empty Cargo package name".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_manifest_outside_checkout() {
        let err = validate_manifest_path("../other/Cargo.toml").unwrap_err();
        assert!(matches!(err, DenError::ValidationError(_)));
    }
}
