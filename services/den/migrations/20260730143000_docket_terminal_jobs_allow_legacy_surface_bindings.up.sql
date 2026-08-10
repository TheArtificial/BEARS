-- Existing jobs without a managed work-surface binding predate the surface
-- requirement. They cannot be dispatched, but users must be able to close them.
ALTER TABLE bear_jobs
    DROP CONSTRAINT IF EXISTS bear_jobs_work_surface_binding;

ALTER TABLE bear_jobs
    ADD CONSTRAINT bear_jobs_work_surface_binding
    CHECK (
        status IN ('completed', 'cancelled', 'archived')
        OR (
            work_surface_id IS NOT NULL
            AND work_surface_ref IS NOT NULL
            AND btrim(work_surface_ref) <> ''
        )
    ) NOT VALID;
