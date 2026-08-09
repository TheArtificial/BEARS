-- `paused` is an active work-run state needed while a human edits its task.
-- This must be a forward migration: 20260804120000 is already applied.
ALTER TABLE bear_work_runs
    DROP CONSTRAINT IF EXISTS bear_work_runs_state_check;

ALTER TABLE bear_work_runs
    ADD CONSTRAINT bear_work_runs_state_check
    CHECK (state IN ('queued', 'claimed', 'provisioning', 'running', 'paused', 'reporting',
                    'succeeded', 'stalled', 'blocked', 'failed', 'cancelled', 'timed_out'));
