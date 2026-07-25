use serde_json::Value;
use sqlx::PgPool;

use crate::{
    config::Config,
    core::tools::{context::DenToolContext, workflow},
    errors::CustomError,
};
use den_core::tools::{
    arguments::PrepareRustDependenciesArguments,
    constants::{
        DEN_JOB_CREATE, DEN_JOB_EVALUATE_CRITERION, DEN_JOB_EXECUTE, DEN_JOB_FIND, DEN_JOB_GET,
        DEN_JOB_LIST, DEN_JOB_UPDATE, DEN_TASK_CREATE, DEN_TASK_FIND, DEN_TASK_LIST,
        DEN_TASK_LISTS_GET_STATUS, DEN_TASK_LISTS_LIST, DEN_TASK_LISTS_UPDATE,
        DEN_TASK_LIST_CHECKOUT, DEN_TASK_LIST_SYNC, DEN_TASK_UPDATE,
        DEN_TASK_UPDATE_CURRENT_STATUS, DEN_WORK_CATALOG, DEN_WORK_DISPATCH,
        DEN_WORK_PREPARE_RUST_DEPENDENCIES, DEN_WORK_RUN_CANCEL, DEN_WORK_RUN_FIND,
        DEN_WORK_RUN_GET, DEN_WORK_RUN_LIST,
    },
};
use den_memory::MemoryStoreManager;
use den_service::bears::BearProfile;
use den_service::conversation::persistence as conversation_persistence;

// The per-call context value now lives in `den-tools` (it is data, not a
// capability), so tool executors can move there. Re-exported here so existing
// `core::tools::session::DenToolInvocationContext` paths and the ~17 in-`den`
// construction sites keep resolving unchanged.
pub use den_core::tools::context::DenToolInvocationContext;

pub async fn invoke_den_tool(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    tool_name: &str,
    arguments: Value,
    context: DenToolInvocationContext,
) -> Result<Value, CustomError> {
    if tool_name == DEN_WORK_PREPARE_RUST_DEPENDENCIES {
        let arguments: PrepareRustDependenciesArguments = serde_json::from_value(arguments)
            .map_err(|error| CustomError::ValidationError(error.to_string()))?;
        tracing::info!(
            audit_event = "rust_dependency_preparation",
            stage = "authorize",
            outcome = "started",
            bear_id = %context.bear_id,
            conversation_id = %context.conversation_id,
            session_id = %context.session_id,
            work_run_id = ?context.work_run_id,
            manifest_path = %arguments.manifest_path,
            package = %arguments.package,
            resolution = ?arguments.resolution,
            preparation = ?arguments.preparation,
            "Rust dependency preparation broker invocation"
        );
        let report_request = serde_json::json!({
            "manifest_path": arguments.manifest_path.clone(),
            "package": arguments.package.clone(),
            "resolution": arguments.resolution.clone(),
            "preparation": arguments.preparation.clone(),
        });
        let runner = SandboxRustDependencyPreparationRunner {
            pool,
            config,
            bear_id: context.bear_id,
        };
        let result = den_service::rust_dependencies::execute_prepare_rust_dependencies(
            &runner, &context, arguments,
        )
        .await;
        match &result {
            Ok(result) => tracing::info!(
                audit_event = "rust_dependency_preparation",
                stage = %result.stage,
                outcome = %result.status,
                code = %result.code,
                retryable = result.retryable,
                lockfile_changed = result.lockfile_changed,
                bear_id = %context.bear_id,
                conversation_id = %context.conversation_id,
                session_id = %context.session_id,
                work_run_id = ?context.work_run_id,
                "Rust dependency preparation broker completed"
            ),
            Err(error) => tracing::warn!(
                audit_event = "rust_dependency_preparation",
                stage = "dispatch",
                outcome = "error",
                error = %error,
                bear_id = %context.bear_id,
                conversation_id = %context.conversation_id,
                session_id = %context.session_id,
                work_run_id = ?context.work_run_id,
                "Rust dependency preparation broker failed"
            ),
        }
        if let (Some(work_run_id), Ok(result)) = (context.work_run_id, result.as_ref()) {
            den_docket::work_runs::record_work_run_dependency_preparation(
                pool,
                work_run_id,
                context.bear_id,
                &serde_json::json!({
                    "status": result.status,
                    "code": result.code,
                    "stage": result.stage,
                    "retryable": result.retryable,
                    "content": result.content,
                    "lockfile_changed": result.lockfile_changed,
                    "manifest_path": report_request["manifest_path"],
                    "package": report_request["package"],
                    "resolution": report_request["resolution"],
                    "preparation": report_request["preparation"],
                }),
            )
            .await
            .map_err(CustomError::from)?;
        }
        return result
            .map(|result| {
                serde_json::to_value(result).expect("Rust dependency result is serializable")
            })
            .map_err(CustomError::from);
    }

    if workflow::is_workflow_tool(tool_name) {
        return invoke_workflow_tool(pool, config, stores, tool_name, arguments, &context).await;
    }

    let ctx = DenToolContext::new(pool, config, stores);
    den_core::tools::dispatch::invoke_den_tool(&ctx, tool_name, arguments, context)
        .await
        .map_err(CustomError::from)
}

/// Den-owned bridge from an authorized work run to its active sandbox provider.
struct SandboxRustDependencyPreparationRunner<'a> {
    pool: &'a PgPool,
    config: &'a Config,
    bear_id: uuid::Uuid,
}

impl den_service::rust_dependencies::RustDependencyPreparationRunner
    for SandboxRustDependencyPreparationRunner<'_>
{
    async fn prepare_rust_dependencies(
        &self,
        request: den_service::rust_dependencies::PrepareRustDependenciesRequest,
    ) -> Result<den_service::rust_dependencies::PrepareRustDependenciesResult, den_core::DenError>
    {
        let run = den_docket::work_runs::get_work_run(self.pool, request.work_run_id)
            .await
            .map_err(|error| den_core::DenError::System(error.to_string()))?
            .filter(|run| run.bear_id == self.bear_id)
            .ok_or_else(|| {
                den_core::DenError::Authorization(
                    "work run is not authorized for this invocation".to_string(),
                )
            })?;
        let sandbox_id = run.sandbox_id.ok_or_else(|| {
            den_core::DenError::ValidationError("work run has no active sandbox".to_string())
        })?;
        let url = self
            .config
            .sandbox_server_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| {
                den_core::DenError::System("sandbox provider is not configured".to_string())
            })?;
        let client = den_sandbox::SandboxClient::new(url, &self.config.sandbox_server_token);
        let response = client
            .prepare_rust_dependencies(
                &sandbox_id,
                &den_sandbox::protocol::PrepareRustDependenciesRequest {
                    manifest_path: request.manifest_path,
                    package: request.package,
                    resolution: match request.resolution {
                        den_core::tools::arguments::RustDependencyResolution::Locked => {
                            den_sandbox::protocol::RustDependencyResolution::Locked
                        }
                        den_core::tools::arguments::RustDependencyResolution::UpdateLockfile => {
                            den_sandbox::protocol::RustDependencyResolution::UpdateLockfile
                        }
                    },
                    preparation: match request.preparation {
                        den_core::tools::arguments::RustDependencyPreparation::Check => {
                            den_sandbox::protocol::RustDependencyPreparation::Check
                        }
                        den_core::tools::arguments::RustDependencyPreparation::TestNoRun => {
                            den_sandbox::protocol::RustDependencyPreparation::TestNoRun
                        }
                    },
                },
            )
            .await
            .map_err(|error| den_core::DenError::System(error.to_string()))?;
        Ok(
            den_service::rust_dependencies::PrepareRustDependenciesResult {
                status: response.status,
                code: response.code,
                stage: response.stage,
                retryable: response.retryable,
                content: response.content,
                lockfile_changed: response.lockfile_changed,
            },
        )
    }
}

async fn invoke_workflow_tool(
    pool: &PgPool,
    config: &Config,
    stores: &MemoryStoreManager,
    tool_name: &str,
    arguments: Value,
    context: &DenToolInvocationContext,
) -> Result<Value, CustomError> {
    reject_closed_freeform_task_definition(tool_name, context)?;
    reject_immutable_focused_task_definition(tool_name, context)?;

    let role = context.profile.unwrap_or(BearProfile::Pair);
    let value = match tool_name {
        DEN_TASK_LISTS_LIST => {
            workflow::list_task_lists(
                pool,
                config,
                stores,
                context,
                role,
                arguments,
                crate::core::tools::activity_payloads::activity_payload,
                crate::core::tools::activity_payloads::plan_mode_workplan_payload,
            )
            .await?
        }
        DEN_TASK_LISTS_GET_STATUS => {
            workflow::get_task_list_status(
                pool,
                context,
                role,
                arguments,
                crate::core::tools::activity_payloads::activity_payload,
            )
            .await?
        }
        DEN_TASK_LISTS_UPDATE => {
            workflow::update_task_list(
                pool,
                context,
                role,
                arguments,
                crate::core::tools::activity_payloads::activity_payload,
            )
            .await?
        }
        DEN_JOB_CREATE => workflow::create_job(pool, context, role, arguments).await?,
        DEN_JOB_LIST => workflow::list_jobs(pool, context, arguments).await?,
        DEN_JOB_GET => workflow::get_job(pool, context, arguments).await?,
        DEN_JOB_FIND => workflow::find_job(pool, context, arguments).await?,
        DEN_JOB_UPDATE => workflow::update_job(pool, context, role, arguments).await?,
        DEN_JOB_EXECUTE => workflow::execute_job(pool, context, role, arguments).await?,
        DEN_JOB_EVALUATE_CRITERION => {
            workflow::evaluate_criterion(pool, context, role, arguments).await?
        }
        DEN_TASK_CREATE => workflow::create_task(pool, context, role, arguments).await?,
        DEN_TASK_LIST => workflow::list_tasks(pool, context, role, arguments).await?,
        DEN_TASK_FIND => workflow::find_task(pool, context, arguments).await?,
        DEN_TASK_UPDATE => workflow::update_task(pool, context, role, arguments).await?,
        DEN_TASK_UPDATE_CURRENT_STATUS => {
            workflow::update_current_task_status(pool, context, role, arguments).await?
        }
        DEN_TASK_LIST_SYNC => workflow::sync_task_list(pool, arguments).await?,
        DEN_TASK_LIST_CHECKOUT => {
            workflow::checkout_task_list(pool, context, role, arguments).await?
        }
        DEN_WORK_DISPATCH => workflow::dispatch_work(pool, context, role, arguments).await?,
        DEN_WORK_RUN_LIST => workflow::list_work_runs(pool, context, arguments).await?,
        DEN_WORK_RUN_GET => workflow::get_work_run(pool, context, arguments).await?,
        DEN_WORK_RUN_FIND => workflow::find_work_run(pool, context, arguments).await?,
        DEN_WORK_RUN_CANCEL => workflow::cancel_work_run(pool, context, role, arguments).await?,
        DEN_WORK_CATALOG => workflow::get_work_catalog(pool, config, context, arguments).await?,
        _ => {
            return Err(CustomError::NotFound(format!(
                "unknown workflow tool: {tool_name}"
            )));
        }
    };

    Ok(value)
}

fn reject_closed_freeform_task_definition(
    tool_name: &str,
    context: &DenToolInvocationContext,
) -> Result<(), CustomError> {
    if closed_freeform_disallows_task_definition(tool_name, context.runtime.as_ref()) {
        return Err(CustomError::ValidationError(
            "task definition tools are unavailable in closed freeform orientation; focus or orient to a task before defining durable work"
                .to_string(),
        ));
    }

    Ok(())
}

fn reject_immutable_focused_task_definition(
    tool_name: &str,
    context: &DenToolInvocationContext,
) -> Result<(), CustomError> {
    if immutable_focused_disallows_task_definition(tool_name, context.runtime.as_ref()) {
        return Err(CustomError::ValidationError(
            "task definition tools are unavailable in immutable focused orientation; use status/result tools or switch to mutable focused work before changing task definitions"
                .to_string(),
        ));
    }

    Ok(())
}

fn task_definition_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        DEN_JOB_CREATE | DEN_TASK_CREATE | DEN_TASK_UPDATE | DEN_TASK_LIST_SYNC
    )
}

fn closed_freeform_disallows_task_definition(tool_name: &str, runtime: Option<&Value>) -> bool {
    if !task_definition_tool(tool_name) {
        return false;
    }

    let Some(orientation) = runtime.and_then(|runtime| runtime.get("objective_orientation")) else {
        return false;
    };

    orientation.get("kind").and_then(Value::as_str) == Some("freeform")
        && orientation
            .get("policy")
            .and_then(|policy| policy.get("may_define_task"))
            .and_then(Value::as_bool)
            == Some(false)
}

fn immutable_focused_disallows_task_definition(tool_name: &str, runtime: Option<&Value>) -> bool {
    if !task_definition_tool(tool_name) {
        return false;
    }

    let Some(orientation) = runtime.and_then(|runtime| runtime.get("objective_orientation")) else {
        return false;
    };

    orientation.get("kind").and_then(Value::as_str) == Some("focused")
        && orientation
            .get("job")
            .and_then(|job| job.get("mutable"))
            .and_then(Value::as_bool)
            == Some(false)
}

pub(crate) struct DenConversationTitleOps<'a> {
    pub(crate) pool: &'a PgPool,
}

impl den_core::tools::conversation::ConversationTitleOps for DenConversationTitleOps<'_> {
    async fn set_title(
        &self,
        bear_id: uuid::Uuid,
        conversation_id: &str,
        title: &str,
    ) -> Result<u64, crate::errors::DenError> {
        conversation_persistence::set_conversation_title_and_sync_client_sessions(
            self.pool,
            bear_id,
            conversation_id,
            title,
        )
        .await
    }
}

#[cfg(test)]
mod test;

#[cfg(test)]
mod orientation_policy_tests {
    use super::*;

    fn closed_freeform_runtime() -> Value {
        serde_json::json!({
            "objective_orientation": {
                "kind": "freeform",
                "policy": { "may_define_task": false }
            }
        })
    }

    fn focused_runtime(mutable: bool) -> Value {
        serde_json::json!({
            "objective_orientation": {
                "kind": "focused",
                "job": {
                    "job_id": "job-1",
                    "active_task_ref": null,
                    "mutable": mutable
                }
            }
        })
    }

    #[test]
    fn closed_freeform_rejects_task_definition_but_allows_status_updates() {
        let runtime = closed_freeform_runtime();

        assert!(closed_freeform_disallows_task_definition(
            DEN_TASK_CREATE,
            Some(&runtime)
        ));
        assert!(!closed_freeform_disallows_task_definition(
            DEN_TASK_UPDATE_CURRENT_STATUS,
            Some(&runtime)
        ));
    }

    #[test]
    fn immutable_focused_rejects_task_definition_but_allows_mutable_and_status_updates() {
        let immutable_runtime = focused_runtime(false);
        let mutable_runtime = focused_runtime(true);

        assert!(immutable_focused_disallows_task_definition(
            DEN_TASK_CREATE,
            Some(&immutable_runtime)
        ));
        assert!(!immutable_focused_disallows_task_definition(
            DEN_TASK_CREATE,
            Some(&mutable_runtime)
        ));
        assert!(!immutable_focused_disallows_task_definition(
            DEN_TASK_UPDATE_CURRENT_STATUS,
            Some(&immutable_runtime)
        ));
    }
}
