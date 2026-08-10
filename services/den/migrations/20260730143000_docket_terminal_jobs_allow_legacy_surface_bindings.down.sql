ALTER TABLE bear_jobs
    DROP CONSTRAINT IF EXISTS bear_jobs_work_surface_binding;

ALTER TABLE bear_jobs
    ADD CONSTRAINT bear_jobs_work_surface_binding
    CHECK (
        work_surface_id IS NOT NULL
        AND work_surface_ref IS NOT NULL
        AND btrim(work_surface_ref) <> ''
    ) NOT VALID;
