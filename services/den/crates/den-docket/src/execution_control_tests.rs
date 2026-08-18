use uuid::Uuid;

use crate::{
    model::DocketExecutionTaskControl, DocketExecutionBinding, DocketExecutionControl,
    DocketExecutionDisposition, DocketExecutionGate, DocketExecutionNextAction,
    DocketExecutionReason,
};

fn control(
    next_action: DocketExecutionNextAction,
    reason: Option<DocketExecutionReason>,
    claimed_task_id: Option<Uuid>,
) -> DocketExecutionControl {
    DocketExecutionControl {
        run_id: Uuid::new_v4(),
        run_state: "running".to_owned(),
        task: DocketExecutionTaskControl {
            selected_task_id: claimed_task_id,
            focused_task_id: claimed_task_id,
            claimed_task_id,
            current_task_id: claimed_task_id,
        },
        next_action,
        retryable: false,
        reason,
    }
}

#[test]
fn execution_control_gate_allows_the_persisted_task_claim() {
    let task_id = Uuid::new_v4();
    let control = control(
        DocketExecutionNextAction::WorkCurrentTask,
        None,
        Some(task_id),
    );
    assert_eq!(
        control.gate(),
        DocketExecutionGate::Allowed {
            task_id,
            binding: DocketExecutionBinding::PairSession {
                job_run_id: control.run_id,
            },
        }
    );
}

#[test]
fn execution_control_gate_rejects_stale_task_claims() {
    assert_eq!(
        control(
            DocketExecutionNextAction::ReconcileExecution,
            Some(DocketExecutionReason::ActiveTaskIsStale),
            Some(Uuid::new_v4()),
        )
        .gate(),
        DocketExecutionGate::Rejected {
            reason: DocketExecutionReason::ActiveTaskIsStale,
            disposition: DocketExecutionDisposition::Reconcile,
        }
    );
}
