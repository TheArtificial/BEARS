-- A stalled work run preserves watchdog evidence for operator resolution instead
-- of fabricating a failed/blocked terminal outcome.
ALTER TABLE bear_work_runs
    DROP CONSTRAINT IF EXISTS bear_work_runs_state_check;

ALTER TABLE bear_work_runs
    ADD CONSTRAINT bear_work_runs_state_check
    CHECK (state IN ('queued', 'claimed', 'provisioning', 'running', 'paused', 'reporting',
                    'succeeded', 'stalled', 'blocked', 'failed', 'cancelled', 'timed_out'));

ALTER TABLE bear_job_runs
    DROP CONSTRAINT IF EXISTS bear_job_runs_state_check;

ALTER TABLE bear_job_runs
    ADD CONSTRAINT bear_job_runs_state_check
    CHECK (state IN ('dispatched', 'running', 'paused', 'stalled', 'blocked',
                    'completed', 'failed', 'cancelled'));
