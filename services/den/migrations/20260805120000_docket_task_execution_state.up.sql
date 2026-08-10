-- A work run is job-scoped, but it may execute one selected task at a time.
-- Keep that association on the run; task state records durable outcomes only.
ALTER TABLE bear_work_runs
    ADD COLUMN IF NOT EXISTS executing_task_id UUID NULL
    REFERENCES bear_tasks (id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_bear_work_runs_executing_task_active
    ON bear_work_runs (executing_task_id)
    WHERE state IN ('queued', 'claimed', 'provisioning', 'running', 'paused', 'reporting');

-- `in_progress` was an execution cache. A run proves activity now, so legacy
-- rows become pending unless they already have a durable terminal outcome.
UPDATE bear_task_run_state
SET status = 'pending',
    started_at = NULL,
    updated_at = NOW()
WHERE status = 'in_progress';

ALTER TABLE bear_task_run_state
    DROP CONSTRAINT IF EXISTS bear_task_run_state_status_check;
ALTER TABLE bear_task_run_state
    ADD CONSTRAINT bear_task_run_state_status_check
    CHECK (status IN ('pending', 'done', 'blocked', 'cancelled'));

DROP INDEX IF EXISTS idx_bear_task_run_state_one_in_progress;
DROP INDEX IF EXISTS bear_task_run_state_one_in_progress_per_run;
