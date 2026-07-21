DROP INDEX IF EXISTS idx_bear_work_runs_one_active_per_job;
ALTER TABLE bear_work_runs ADD COLUMN IF NOT EXISTS task_id UUID NULL REFERENCES bear_tasks (id) ON DELETE CASCADE;
CREATE UNIQUE INDEX IF NOT EXISTS idx_bear_work_runs_one_active_per_task
    ON bear_work_runs (task_id)
    WHERE task_id IS NOT NULL
      AND state IN ('queued', 'claimed', 'provisioning', 'running', 'reporting');
