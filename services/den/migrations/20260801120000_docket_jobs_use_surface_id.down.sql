ALTER TABLE bear_jobs
DROP CONSTRAINT IF EXISTS bear_jobs_work_surface_binding;

ALTER TABLE bear_jobs
DROP CONSTRAINT IF EXISTS bear_jobs_work_surface_id_fkey;

ALTER TABLE bear_jobs
ADD CONSTRAINT bear_jobs_work_surface_id_fkey
FOREIGN KEY (work_surface_id) REFERENCES work_surfaces(id) ON DELETE SET NULL;

ALTER TABLE bear_jobs
ADD COLUMN work_surface_ref TEXT;

UPDATE bear_jobs AS jobs
SET work_surface_ref = surfaces.name
FROM work_surfaces AS surfaces
WHERE surfaces.id = jobs.work_surface_id;

ALTER TABLE bear_jobs
ADD CONSTRAINT bear_jobs_work_surface_binding
CHECK (
    status IN ('completed', 'cancelled', 'archived')
    OR (work_surface_id IS NOT NULL AND NULLIF(btrim(work_surface_ref), '') IS NOT NULL)
)
NOT VALID;
