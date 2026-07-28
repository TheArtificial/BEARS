ALTER TABLE bear_jobs
    ADD COLUMN supersedes_job_id UUID NULL REFERENCES bear_jobs (id) ON DELETE SET NULL;

CREATE INDEX idx_bear_jobs_supersedes
    ON bear_jobs (supersedes_job_id)
    WHERE supersedes_job_id IS NOT NULL;
