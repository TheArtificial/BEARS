-- A terminal work run can block its owning Docket run without cancelling it.
ALTER TABLE bear_job_runs
    DROP CONSTRAINT IF EXISTS bear_job_runs_state_check;

ALTER TABLE bear_job_runs
    ADD CONSTRAINT bear_job_runs_state_check
    CHECK (state IN ('dispatched', 'running', 'paused', 'blocked', 'completed', 'failed', 'cancelled'));
