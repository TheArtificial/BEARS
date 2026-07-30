-- Resolve legacy text bindings only when the named managed surface is assigned
-- to the job's Bear. Unmatched jobs remain explicitly unbound for repair.
UPDATE bear_jobs AS jobs
SET work_surface_id = surfaces.id
FROM work_surfaces AS surfaces
JOIN work_surface_bears AS assignments
    ON assignments.surface_id = surfaces.id
WHERE jobs.work_surface_id IS NULL
  AND assignments.bear_id = jobs.bear_id
  AND surfaces.name = btrim(jobs.work_surface_ref);

ALTER TABLE bear_jobs
DROP CONSTRAINT IF EXISTS bear_jobs_work_surface_binding;

-- A job's canonical surface relationship must remain valid for history and
-- dispatch. Archive surfaces instead of deleting referenced records.
ALTER TABLE bear_jobs
DROP CONSTRAINT IF EXISTS bear_jobs_work_surface_id_fkey;

ALTER TABLE bear_jobs
ADD CONSTRAINT bear_jobs_work_surface_id_fkey
FOREIGN KEY (work_surface_id) REFERENCES work_surfaces(id) ON DELETE RESTRICT;

ALTER TABLE bear_jobs
DROP COLUMN work_surface_ref;

ALTER TABLE bear_jobs
ADD CONSTRAINT bear_jobs_work_surface_binding
CHECK (status IN ('completed', 'cancelled', 'archived') OR work_surface_id IS NOT NULL)
NOT VALID;
