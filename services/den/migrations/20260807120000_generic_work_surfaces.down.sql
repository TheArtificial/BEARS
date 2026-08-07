-- This migration only introduced the git_workspace kind, so every registry
-- row must still have a Git detail record before the old shape can be restored.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM work_surfaces surfaces
        LEFT JOIN git_work_surface_details git ON git.id = surfaces.id
        WHERE git.id IS NULL
    ) THEN
        RAISE EXCEPTION 'cannot revert generic work surfaces after non-Git surfaces exist';
    END IF;
END $$;

ALTER TABLE bear_jobs ADD COLUMN work_surface_id UUID NULL;
UPDATE bear_jobs jobs
SET work_surface_id = assignments.work_surface_id
FROM job_work_surface_assignments assignments
WHERE assignments.job_id = jobs.id;

ALTER TABLE bear_jobs
    ADD CONSTRAINT bear_jobs_work_surface_id_fkey
        FOREIGN KEY (work_surface_id) REFERENCES git_work_surface_details (id) ON DELETE RESTRICT;
ALTER TABLE bear_jobs
    ADD CONSTRAINT bear_jobs_work_surface_binding
        CHECK (status IN ('completed', 'cancelled', 'archived') OR work_surface_id IS NOT NULL)
        NOT VALID;

DROP TABLE job_work_surface_assignments;

ALTER TABLE work_surface_managers
    DROP CONSTRAINT work_surface_managers_surface_id_fkey,
    ADD CONSTRAINT work_surface_managers_surface_id_fkey
        FOREIGN KEY (surface_id) REFERENCES git_work_surface_details (id) ON DELETE CASCADE;
ALTER TABLE work_surface_bears
    DROP CONSTRAINT work_surface_bears_surface_id_fkey,
    ADD CONSTRAINT work_surface_bears_surface_id_fkey
        FOREIGN KEY (surface_id) REFERENCES git_work_surface_details (id) ON DELETE CASCADE;

ALTER TABLE git_work_surface_details
    ADD COLUMN name TEXT,
    ADD COLUMN description TEXT,
    ADD COLUMN created_by_user_id INTEGER REFERENCES users (id) ON DELETE RESTRICT,
    ADD COLUMN created_at TIMESTAMPTZ,
    ADD COLUMN updated_at TIMESTAMPTZ;
UPDATE git_work_surface_details git
SET name = surfaces.name,
    description = surfaces.description,
    created_by_user_id = surfaces.created_by_user_id,
    created_at = surfaces.created_at,
    updated_at = surfaces.updated_at
FROM work_surfaces surfaces
WHERE surfaces.id = git.id;
ALTER TABLE git_work_surface_details
    ALTER COLUMN name SET NOT NULL,
    ALTER COLUMN created_by_user_id SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL,
    ADD CONSTRAINT work_surfaces_name_key UNIQUE (name);

DROP TABLE work_surfaces;
ALTER TABLE git_work_surface_details RENAME TO work_surfaces;
