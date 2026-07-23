//! Sandbox type selection/validation policy: a small, pure, visible rule
//! table. Every rejection carries an actionable reason; nothing is silently
//! degraded to a weaker boundary.

use crate::protocol::{CreateSandboxRequest, SandboxType};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("sandbox type '{requested}' is not implemented; supported: container")]
    UnimplementedType { requested: &'static str },
    #[error("task requires write access but requested a readonly sandbox type")]
    ReadonlyWriteConflict,
    #[error("container runtime unavailable on this sandbox host: {reason}")]
    RuntimeUnavailable { reason: String },
    #[error("sandbox capacity reached ({active}/{max} active); retry later")]
    QueueFull { active: usize, max: usize },
    #[error("unknown root '{name}'; configure it in the roots file")]
    UnknownRoot { name: String },
}

pub struct PolicyContext {
    pub backend_available: bool,
    pub active_sandboxes: usize,
    pub max_concurrent: usize,
    pub root_known: bool,
}

pub fn validate_selection(
    request: &CreateSandboxRequest,
    ctx: &PolicyContext,
) -> Result<(), PolicyError> {
    if !ctx.root_known {
        return Err(PolicyError::UnknownRoot {
            name: request.root.clone(),
        });
    }
    match request.sandbox_type {
        SandboxType::Container => {}
        other => {
            return Err(PolicyError::UnimplementedType {
                requested: other.as_str(),
            })
        }
    }
    if request.requires_write && request.sandbox_type == SandboxType::LocalWorkspaceReadonly {
        return Err(PolicyError::ReadonlyWriteConflict);
    }
    if !ctx.backend_available {
        return Err(PolicyError::RuntimeUnavailable {
            reason: "docker probe failed (is the daemon running and reachable via \
                     DOCKER_HOST or the local socket?)"
                .to_string(),
        });
    }
    if ctx.active_sandboxes >= ctx.max_concurrent {
        return Err(PolicyError::QueueFull {
            active: ctx.active_sandboxes,
            max: ctx.max_concurrent,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(sandbox_type: SandboxType, requires_write: bool) -> CreateSandboxRequest {
        CreateSandboxRequest {
            root: "demo".into(),
            git_ref: None,
            sandbox_type,
            requires_write,
            image: None,
            network: Default::default(),
            prepare_cargo_dependencies: false,
            env: Default::default(),
            limits: Default::default(),
            labels: Default::default(),
        }
    }

    fn ok_ctx() -> PolicyContext {
        PolicyContext {
            backend_available: true,
            active_sandboxes: 0,
            max_concurrent: 2,
            root_known: true,
        }
    }

    #[test]
    fn container_with_capacity_passes() {
        assert_eq!(
            validate_selection(&request(SandboxType::Container, true), &ok_ctx()),
            Ok(())
        );
    }

    #[test]
    fn every_unimplemented_type_is_rejected_explicitly() {
        for sandbox_type in [
            SandboxType::LocalWorkspaceReadonly,
            SandboxType::LocalWorkspaceWritable,
            SandboxType::EphemeralCopy,
            SandboxType::RemoteEphemeral,
        ] {
            let err = validate_selection(&request(sandbox_type, false), &ok_ctx()).unwrap_err();
            assert_eq!(
                err,
                PolicyError::UnimplementedType {
                    requested: sandbox_type.as_str()
                }
            );
        }
    }

    #[test]
    fn unknown_root_rejected_before_type_check() {
        let mut ctx = ok_ctx();
        ctx.root_known = false;
        assert!(matches!(
            validate_selection(&request(SandboxType::RemoteEphemeral, false), &ctx),
            Err(PolicyError::UnknownRoot { .. })
        ));
    }

    #[test]
    fn missing_runtime_blocks_with_reason() {
        let mut ctx = ok_ctx();
        ctx.backend_available = false;
        assert!(matches!(
            validate_selection(&request(SandboxType::Container, true), &ctx),
            Err(PolicyError::RuntimeUnavailable { .. })
        ));
    }

    #[test]
    fn capacity_limit_reports_queue_full() {
        let mut ctx = ok_ctx();
        ctx.active_sandboxes = 2;
        assert_eq!(
            validate_selection(&request(SandboxType::Container, true), &ctx),
            Err(PolicyError::QueueFull { active: 2, max: 2 })
        );
    }
}
