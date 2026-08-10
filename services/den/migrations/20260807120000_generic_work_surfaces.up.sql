-- A work surface is a generic managed target. Git provisioning details belong in
-- the typed adapter table, not in the registry. Existing UUIDs are retained so
-- grants and historical references can be backfilled without identity remapping.
ALTER TABLE work_surfaces RENAME TO git_work_surface_details;
-- Constraint names are schema-wide; release the old Git table's name before
-- the generic registry receives its own unique name constraint.
ALTER TABLE git_work_surface_details DROP CONSTRAINT work_surfaces_name_key;

CREATE TABLE work_surfaces (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL CHECK (name ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    description TEXT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('git_workspace')),
    created_by_user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT work_surfaces_name_key UNIQUE (name)
);

INSERT INTO work_surfaces (id, name, description, kind, created_by_user_id, created_at, updated_at)
SELECT id, name, description, 'git_workspace', created_by_user_id, created_at, updated_at
FROM git_work_surface_details;

ALTER TABLE git_work_surface_details
    ADD CONSTRAINT git_work_surface_details_id_fkey
        FOREIGN KEY (id) REFERENCES work_surfaces (id) ON DELETE CASCADE;

-- Grants apply to the generic surface, not a particular adapter implementation.
ALTER TABLE work_surface_managers
    DROP CONSTRAINT work_surface_managers_surface_id_fkey,
    ADD CONSTRAINT work_surface_managers_surface_id_fkey
        FOREIGN KEY (surface_id) REFERENCES work_surfaces (id) ON DELETE CASCADE;
ALTER TABLE work_surface_bears
    DROP CONSTRAINT work_surface_bears_surface_id_fkey,
    ADD CONSTRAINT work_surface_bears_surface_id_fkey
        FOREIGN KEY (surface_id) REFERENCES work_surfaces (id) ON DELETE CASCADE;

CREATE TABLE job_work_surface_assignments (
    job_id UUID NOT NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    work_surface_id UUID NOT NULL REFERENCES work_surfaces (id) ON DELETE RESTRICT,
    mutation_policy TEXT NOT NULL DEFAULT 'required'
        CHECK (mutation_policy IN ('required', 'optional', 'forbidden')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, work_surface_id)
);

INSERT INTO job_work_surface_assignments (job_id, work_surface_id, mutation_policy)
SELECT id, work_surface_id, 'required'
FROM bear_jobs
WHERE work_surface_id IS NOT NULL;

ALTER TABLE bear_jobs
    DROP CONSTRAINT IF EXISTS bear_jobs_work_surface_binding,
    DROP CONSTRAINT IF EXISTS bear_jobs_work_surface_id_fkey,
    DROP COLUMN work_surface_id;

ALTER TABLE git_work_surface_details
    DROP CONSTRAINT IF EXISTS work_surfaces_name_key,
    DROP COLUMN name,
    DROP COLUMN description,
    DROP COLUMN created_by_user_id,
    DROP COLUMN created_at,
    DROP COLUMN updated_at;

CREATE INDEX idx_job_work_surface_assignments_surface
    ON job_work_surface_assignments (work_surface_id);
