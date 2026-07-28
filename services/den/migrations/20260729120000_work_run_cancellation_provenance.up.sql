ALTER TABLE bear_work_runs
    ADD COLUMN IF NOT EXISTS cancel_requested_by TEXT NULL,
    ADD COLUMN IF NOT EXISTS cancel_reason TEXT NULL,
    ADD COLUMN IF NOT EXISTS cancel_requested_at TIMESTAMPTZ NULL;
