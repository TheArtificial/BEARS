DROP INDEX IF EXISTS idx_bear_work_runs_attached_session;

ALTER TABLE bear_work_runs
    DROP CONSTRAINT IF EXISTS bear_work_runs_attachment_state_check,
    DROP CONSTRAINT IF EXISTS bear_work_runs_attachment_check,
    DROP CONSTRAINT IF EXISTS bear_work_runs_execution_target_check,
    DROP COLUMN IF EXISTS disconnect_deadline_at,
    DROP COLUMN IF EXISTS disconnected_at,
    DROP COLUMN IF EXISTS attachment_warning,
    DROP COLUMN IF EXISTS attachment_state,
    DROP COLUMN IF EXISTS attached_client_session_id,
    DROP COLUMN IF EXISTS execution_target;
