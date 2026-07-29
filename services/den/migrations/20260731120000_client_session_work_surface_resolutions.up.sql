-- Canonical, server-owned work-surface resolution for a client session.
-- Adapter environment remains untrusted input evidence and is never used as
-- durable work-surface identity.

CREATE TABLE client_session_work_surface_resolutions (
    client_session_id UUID PRIMARY KEY REFERENCES client_sessions (id) ON DELETE CASCADE,
    work_surface_id UUID NOT NULL REFERENCES work_surfaces (id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('resolved', 'confirmed')),
    evidence JSONB NOT NULL DEFAULT '{}'::jsonb,
    resolved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_client_session_work_surface_resolutions_surface
    ON client_session_work_surface_resolutions (work_surface_id);

COMMENT ON TABLE client_session_work_surface_resolutions IS
    'Server-owned canonical work-surface identity for a client session. Adapter environment is input evidence only.';
