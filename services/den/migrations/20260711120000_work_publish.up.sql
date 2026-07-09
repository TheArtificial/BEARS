-- Work-run publishing: jobs carry the upstream branch their runs push to
-- (caller-specified; generated den/job-<short-id> on first pushable dispatch),
-- and runs record which catalog image they were dispatched with.

ALTER TABLE bear_jobs ADD COLUMN IF NOT EXISTS work_branch TEXT NULL;
ALTER TABLE bear_work_runs ADD COLUMN IF NOT EXISTS image_name TEXT NULL;
