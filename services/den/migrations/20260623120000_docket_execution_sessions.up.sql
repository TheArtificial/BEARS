-- Session-scoped Docket execution bindings.
-- This records when a stance/session is actively executing a Docket job/task so
-- prompt assembly can distinguish "doing the job" from discussing/planning it.

CREATE TABLE IF NOT EXISTS docket_execution_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    owner_profile TEXT NOT NULL CHECK (owner_profile IN ('chat', 'pair', 'curate', 'work', 'watch')),
    session_id TEXT NOT NULL,
    source_conversation_id TEXT NULL,
    source_acp_session_id TEXT NULL,
    job_id UUID NOT NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    task_id UUID NULL REFERENCES bear_tasks (id) ON DELETE SET NULL,
    state TEXT NOT NULL DEFAULT 'active'
        CHECK (state IN ('active', 'blocked', 'completing', 'paused', 'completed', 'cancelled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(session_id) <> '')
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_docket_execution_sessions_active_session
    ON docket_execution_sessions (bear_id, owner_profile, session_id)
    WHERE state IN ('active', 'blocked', 'completing', 'paused');

CREATE INDEX IF NOT EXISTS idx_docket_execution_sessions_job_run
    ON docket_execution_sessions (job_id, run_id, state, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_docket_execution_sessions_acp
    ON docket_execution_sessions (bear_id, source_acp_session_id, state, updated_at DESC)
    WHERE source_acp_session_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_docket_execution_sessions_conversation
    ON docket_execution_sessions (bear_id, source_conversation_id, state, updated_at DESC)
    WHERE source_conversation_id IS NOT NULL;

COMMENT ON TABLE docket_execution_sessions IS 'Session/stance-local Docket execution mode bindings; separates active job execution from ordinary planning/chat.';
