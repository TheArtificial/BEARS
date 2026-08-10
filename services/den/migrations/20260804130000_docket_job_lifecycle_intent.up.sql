-- Job operational state is a derived projection (ADR-0034). Persist only
-- explicit lifecycle intent; work-run, task, and criterion evidence determines
-- draft/ready/running/stalled/blocked/completed.
ALTER TABLE bear_jobs
    ADD COLUMN IF NOT EXISTS lifecycle_intent TEXT NULL
        CHECK (lifecycle_intent IN ('cancelled', 'archived'));

UPDATE bear_jobs
SET lifecycle_intent = status
WHERE status IN ('cancelled', 'archived');

DROP INDEX IF EXISTS idx_bear_jobs_bear_status;
ALTER TABLE bear_jobs DROP COLUMN IF EXISTS status;

-- The preceding surface-binding constraint allowed old terminal *statuses*
-- to remain unbound. Lifecycle intent is the only persisted terminal fact.
ALTER TABLE bear_jobs
    DROP CONSTRAINT IF EXISTS bear_jobs_work_surface_binding;
ALTER TABLE bear_jobs
    ADD CONSTRAINT bear_jobs_work_surface_binding
    CHECK (lifecycle_intent IN ('cancelled', 'archived') OR work_surface_id IS NOT NULL)
    NOT VALID;

CREATE INDEX IF NOT EXISTS idx_bear_jobs_bear_lifecycle
    ON bear_jobs (bear_id, lifecycle_intent, updated_at DESC);
