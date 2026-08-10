ALTER TABLE bear_job_runs
    DROP CONSTRAINT IF EXISTS bear_job_runs_state_check;

ALTER TABLE bear_job_runs
    ADD CONSTRAINT bear_job_runs_state_check
    CHECK (state IN ('dispatched', 'running', 'paused', 'completed', 'failed', 'cancelled'));
