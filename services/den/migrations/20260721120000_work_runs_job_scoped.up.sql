-- Work runs are job dispatch attempts, not task dispatch attempts. Task
-- progress remains represented by bear_task_run_state and bear_task_events.
DROP INDEX IF EXISTS idx_bear_work_runs_one_active_per_task;
ALTER TABLE bear_work_runs DROP COLUMN IF EXISTS task_id;
CREATE UNIQUE INDEX IF NOT EXISTS idx_bear_work_runs_one_active_per_job
    ON bear_work_runs (job_id)
    WHERE state IN ('queued', 'claimed', 'provisioning', 'running', 'reporting');
