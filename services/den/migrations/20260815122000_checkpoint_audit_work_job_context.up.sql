-- Opaque adapter-resolved Work/Docket correlation for runtime checkpoint audit artifacts.
ALTER TABLE bear_run_checkpoints
    ADD COLUMN related_work_run_id UUID NULL REFERENCES bear_work_runs(id) ON DELETE SET NULL,
    ADD COLUMN related_docket_job_id UUID NULL REFERENCES bear_jobs(id) ON DELETE SET NULL;

CREATE INDEX idx_bear_run_checkpoints_work_run
    ON bear_run_checkpoints (related_work_run_id, created_at DESC)
    WHERE related_work_run_id IS NOT NULL;

CREATE INDEX idx_bear_run_checkpoints_docket_job
    ON bear_run_checkpoints (related_docket_job_id, created_at DESC)
    WHERE related_docket_job_id IS NOT NULL;
