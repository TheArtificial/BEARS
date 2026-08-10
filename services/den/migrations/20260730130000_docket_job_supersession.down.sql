DROP INDEX IF EXISTS idx_bear_jobs_supersedes;

ALTER TABLE bear_jobs
    DROP COLUMN IF EXISTS supersedes_job_id;
