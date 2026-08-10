ALTER TABLE bear_work_runs
    DROP COLUMN IF EXISTS cancel_requested_at,
    DROP COLUMN IF EXISTS cancel_reason,
    DROP COLUMN IF EXISTS cancel_requested_by;
