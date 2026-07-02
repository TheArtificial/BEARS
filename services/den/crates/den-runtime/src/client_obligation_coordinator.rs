use serde_json::Value;
use sqlx::PgPool;

use den_core::DenError;

use den_core::tools::constants::DEN_WEB_FETCH;

use crate::{turn_obligations, turn_steps, turn_runs};

fn obligation_is_den_web_fetch(obligation_payload: &Value) -> bool {
    let tool_name = obligation_payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    den_core::tools::descriptor::builtin_den_tool_descriptor_for_provider_name(tool_name)
        .map(|descriptor| descriptor.name == DEN_WEB_FETCH)
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub enum ToolResultCoordinatorOutcome {
    WaitingForMoreClientResults {
        run: Option<turn_runs::BearWireRunRow>,
        open_obligations: Vec<turn_obligations::BearWireRunObligationRow>,
    },
    ContinueModel {
        run: Option<turn_runs::BearWireRunRow>,
    },
    IgnoredLateResult {
        run_state: String,
        obligation_state: String,
    },
}

#[derive(Debug, Clone)]
pub enum PermissionResultCoordinatorOutcome {
    DispatchLocalTool {
        run: Option<turn_runs::BearWireRunRow>,
        tool_obligation: turn_obligations::BearWireRunObligationRow,
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ContinueModel {
        run: Option<turn_runs::BearWireRunRow>,
    },
    IgnoredLateResult {
        run_state: String,
        obligation_state: String,
    },
}

pub async fn settle_tool_result(
    pool: &PgPool,
    run: &turn_runs::BearWireRunRow,
    obligation: &turn_obligations::BearWireRunObligationRow,
    result_payload: Value,
) -> Result<ToolResultCoordinatorOutcome, DenError> {
    let Some(_received_obligation) =
        turn_obligations::mark_result_received(pool, obligation.id, result_payload).await?
    else {
        return Ok(ToolResultCoordinatorOutcome::IgnoredLateResult {
            run_state: run.state.clone(),
            obligation_state: obligation.state.clone(),
        });
    };

    let open_obligations = if let Some(step_id) = obligation.turn_step_id {
        turn_obligations::open_client_obligations_for_step(pool, step_id).await?
    } else {
        turn_obligations::open_client_obligations_for_run(pool, &run.run_id).await?
    };
    if !open_obligations.is_empty() {
        let transitioned = turn_runs::transition_run(
            pool,
            &run.run_id,
            turn_runs::BearWireRunState::WaitingForToolResult,
            None,
        )
        .await?;
        return Ok(ToolResultCoordinatorOutcome::WaitingForMoreClientResults {
            run: transitioned,
            open_obligations,
        });
    }

    let transitioned = turn_runs::transition_run(
        pool,
        &run.run_id,
        turn_runs::BearWireRunState::Continuing,
        None,
    )
    .await?;
    let _ = turn_obligations::mark_continued(pool, obligation.id).await?;
    if let Some(step_id) = obligation.turn_step_id {
        let _ = turn_steps::transition_step(pool, step_id, "continued").await?;
    }
    Ok(ToolResultCoordinatorOutcome::ContinueModel { run: transitioned })
}

pub async fn settle_permission_result(
    pool: &PgPool,
    run: &turn_runs::BearWireRunRow,
    obligation: &turn_obligations::BearWireRunObligationRow,
    normalized_decision: &str,
    result_payload: Value,
) -> Result<PermissionResultCoordinatorOutcome, DenError> {
    let Some(_received_obligation) =
        turn_obligations::mark_result_received(pool, obligation.id, result_payload).await?
    else {
        return Ok(PermissionResultCoordinatorOutcome::IgnoredLateResult {
            run_state: run.state.clone(),
            obligation_state: obligation.state.clone(),
        });
    };

    if normalized_decision == "granted" && !obligation_is_den_web_fetch(&obligation.request_payload)
    {
        let Some(tool_call_id) = obligation.tool_call_id.clone() else {
            return Err(DenError::ValidationError(
                "granted armature-local permission obligation missing tool_call_id".to_string(),
            ));
        };
        let Some(tool_obligation) =
            turn_obligations::mark_waiting_for_tool_result(pool, obligation.id).await?
        else {
            return Ok(PermissionResultCoordinatorOutcome::IgnoredLateResult {
                run_state: run.state.clone(),
                obligation_state: obligation.state.clone(),
            });
        };
        let transitioned = turn_runs::transition_run(
            pool,
            &run.run_id,
            turn_runs::BearWireRunState::WaitingForToolResult,
            None,
        )
        .await?;
        let tool_name = obligation
            .request_payload
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("local_tool")
            .to_string();
        let args = obligation
            .request_payload
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        return Ok(PermissionResultCoordinatorOutcome::DispatchLocalTool {
            run: transitioned,
            tool_obligation,
            tool_call_id,
            tool_name,
            args,
        });
    }

    let transitioned = turn_runs::transition_run(
        pool,
        &run.run_id,
        turn_runs::BearWireRunState::Continuing,
        None,
    )
    .await?;
    let _ = turn_obligations::mark_continued(pool, obligation.id).await?;
    if let Some(step_id) = obligation.turn_step_id {
        let _ = turn_steps::transition_step(pool, step_id, "continued").await?;
    }
    Ok(PermissionResultCoordinatorOutcome::ContinueModel { run: transitioned })
}
