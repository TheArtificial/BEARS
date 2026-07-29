-- Docket work jobs are rooted in one managed work surface. Pair task trees
-- live under client-session anchors and are no longer represented by jobs.

DELETE FROM bear_jobs
WHERE objective_kind = 'conversation_task_list';

DROP INDEX IF EXISTS idx_bear_jobs_one_active_conversation_objective;

ALTER TABLE bear_jobs
    ADD CONSTRAINT bear_jobs_work_surface_binding
    CHECK (
        work_surface_id IS NOT NULL
        AND work_surface_ref IS NOT NULL
        AND btrim(work_surface_ref) <> ''
    ) NOT VALID;

ALTER TABLE bear_jobs
    VALIDATE CONSTRAINT bear_jobs_work_surface_binding;
