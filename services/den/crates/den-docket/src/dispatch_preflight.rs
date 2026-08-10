//! Canonical, side-effect-free preflight for Docket work dispatch.

use serde::Serialize;

use crate::{model::DocketCommitPolicy, work_runs::WorkExecutionTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableResultKind {
    Report,
    RepositoryChanges,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutRelationship {
    Isolated,
    Attached,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicationRoute {
    None,
    CommitToBranch { branch: Option<String> },
    AttachedWorktree,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchBlocker {
    RepositoryChangesWithoutPublication,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DispatchPreflight {
    pub execution_target: String,
    pub checkout_relationship: CheckoutRelationship,
    pub durable_result: DurableResultKind,
    pub publication: PublicationRoute,
    pub dispatchable: bool,
    pub blocker: Option<DispatchBlocker>,
}

pub fn preflight_dispatch(
    target: &WorkExecutionTarget,
    durable_result: DurableResultKind,
    commit_policy: Option<DocketCommitPolicy>,
    work_branch: Option<&str>,
) -> DispatchPreflight {
    let checkout_relationship = match target {
        WorkExecutionTarget::Sandbox => CheckoutRelationship::Isolated,
        WorkExecutionTarget::AttachedArmature { .. } => CheckoutRelationship::Attached,
    };
    let publication = match target {
        WorkExecutionTarget::AttachedArmature { .. } => PublicationRoute::AttachedWorktree,
        WorkExecutionTarget::Sandbox
            if matches!(commit_policy, Some(DocketCommitPolicy::PerTask)) =>
        {
            PublicationRoute::CommitToBranch {
                branch: work_branch.map(ToOwned::to_owned),
            }
        }
        WorkExecutionTarget::Sandbox => PublicationRoute::None,
    };
    let blocker = (durable_result == DurableResultKind::RepositoryChanges
        && publication == PublicationRoute::None)
        .then_some(DispatchBlocker::RepositoryChangesWithoutPublication);

    DispatchPreflight {
        execution_target: target.as_str().to_owned(),
        checkout_relationship,
        durable_result,
        publication,
        dispatchable: blocker.is_none(),
        blocker,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_report_without_commits_is_dispatchable() {
        let preflight = preflight_dispatch(
            &WorkExecutionTarget::Sandbox,
            DurableResultKind::Report,
            Some(DocketCommitPolicy::None),
            None,
        );

        assert!(preflight.dispatchable);
        assert_eq!(
            preflight.checkout_relationship,
            CheckoutRelationship::Isolated
        );
        assert_eq!(preflight.publication, PublicationRoute::None);
    }

    #[test]
    fn sandbox_repository_changes_require_publication() {
        let preflight = preflight_dispatch(
            &WorkExecutionTarget::Sandbox,
            DurableResultKind::RepositoryChanges,
            Some(DocketCommitPolicy::None),
            None,
        );

        assert!(!preflight.dispatchable);
        assert_eq!(
            preflight.blocker,
            Some(DispatchBlocker::RepositoryChangesWithoutPublication)
        );
    }

    #[test]
    fn sandbox_repository_changes_publish_to_the_work_branch_per_task() {
        let preflight = preflight_dispatch(
            &WorkExecutionTarget::Sandbox,
            DurableResultKind::RepositoryChanges,
            Some(DocketCommitPolicy::PerTask),
            Some("den/job-1234"),
        );

        assert!(preflight.dispatchable);
        assert_eq!(
            preflight.publication,
            PublicationRoute::CommitToBranch {
                branch: Some("den/job-1234".into())
            }
        );
    }

    #[test]
    fn sandbox_repository_changes_reject_per_job_commits() {
        let preflight = preflight_dispatch(
            &WorkExecutionTarget::Sandbox,
            DurableResultKind::RepositoryChanges,
            Some(DocketCommitPolicy::PerJob),
            Some("den/job-1234"),
        );

        assert!(!preflight.dispatchable);
        assert_eq!(
            preflight.blocker,
            Some(DispatchBlocker::RepositoryChangesWithoutPublication)
        );
    }

    #[test]
    fn attached_repository_changes_modify_the_attached_worktree() {
        let preflight = preflight_dispatch(
            &WorkExecutionTarget::AttachedArmature {
                client_session_id: "session-1".into(),
            },
            DurableResultKind::RepositoryChanges,
            Some(DocketCommitPolicy::None),
            None,
        );

        assert!(preflight.dispatchable);
        assert_eq!(
            preflight.checkout_relationship,
            CheckoutRelationship::Attached
        );
        assert_eq!(preflight.publication, PublicationRoute::AttachedWorktree);
    }
}
