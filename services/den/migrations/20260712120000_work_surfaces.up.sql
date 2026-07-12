-- Managed work surfaces: den-level entities backing sandbox roots.
-- Surfaces are created by users (creator = owner), managed via
-- work_surface_managers grants, and assigned to bears (full access) via
-- work_surface_bears. The name is immutable after creation: it is the
-- provider-side root identity (pristine clone directory) and the value
-- denormalized into bear_jobs.work_surface_ref.

CREATE TABLE work_surfaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    description TEXT NULL,
    upstream_url TEXT NOT NULL CHECK (btrim(upstream_url) <> ''),
    default_ref TEXT NOT NULL DEFAULT 'main' CHECK (btrim(default_ref) <> ''),
    default_image TEXT NULL,
    credential_kind TEXT NULL CHECK (credential_kind IN ('ssh_key', 'https_token')),
    credential_encrypted TEXT NULL,
    created_by_user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((credential_kind IS NULL) = (credential_encrypted IS NULL))
);

CREATE TABLE work_surface_managers (
    surface_id UUID NOT NULL REFERENCES work_surfaces (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'manager' CHECK (role IN ('owner', 'manager')),
    granted_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (surface_id, user_id)
);

CREATE TABLE work_surface_bears (
    surface_id UUID NOT NULL REFERENCES work_surfaces (id) ON DELETE CASCADE,
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    granted_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (surface_id, bear_id)
);

CREATE INDEX idx_work_surface_bears_bear ON work_surface_bears (bear_id);

-- Job history survives surface deletion (SET NULL); the legacy
-- work_surface_ref text column keeps the display name.
ALTER TABLE bear_jobs
    ADD COLUMN work_surface_id UUID NULL REFERENCES work_surfaces (id) ON DELETE SET NULL;

CREATE INDEX idx_bear_jobs_work_surface_id ON bear_jobs (work_surface_id)
    WHERE work_surface_id IS NOT NULL;
