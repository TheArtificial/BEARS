CREATE TABLE IF NOT EXISTS prompt_memory_blocks (
    id BIGSERIAL PRIMARY KEY,
    block_id TEXT NOT NULL UNIQUE,
    bear_id UUID NULL REFERENCES bears (id) ON DELETE CASCADE,
    role_slug TEXT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('bear_wide', 'role_local', 'work_surface', 'session')),
    block_type TEXT NOT NULL CHECK (block_type IN ('role_guidance', 'work_surface_context', 'session_focus', 'user_instruction')),
    state TEXT NOT NULL CHECK (state IN ('draft', 'active', 'superseded', 'archived')),
    work_surface TEXT NULL,
    session_id TEXT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    created_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    supersedes_block_id TEXT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_prompt_memory_blocks_bear_role_scope
    ON prompt_memory_blocks (bear_id, role_slug, scope, state);
CREATE INDEX IF NOT EXISTS idx_prompt_memory_blocks_session
    ON prompt_memory_blocks (session_id, state)
    WHERE session_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_prompt_memory_blocks_work_surface
    ON prompt_memory_blocks (work_surface, state)
    WHERE work_surface IS NOT NULL;

COMMENT ON TABLE prompt_memory_blocks IS 'Den-owned editable prompt memory blocks compiled into runtime context by explicit scope and lifecycle.';
COMMENT ON COLUMN prompt_memory_blocks.block_id IS 'Stable external identifier for prompt memory block revisions.';
COMMENT ON COLUMN prompt_memory_blocks.supersedes_block_id IS 'Previous prompt memory block identifier explicitly superseded by this row when meaning materially changes.';
