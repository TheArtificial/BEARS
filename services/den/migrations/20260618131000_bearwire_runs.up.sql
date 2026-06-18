CREATE TABLE IF NOT EXISTS bearwire_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL,
    bear_id UUID NOT NULL REFERENCES bears(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    state TEXT NOT NULL DEFAULT 'accepted' CHECK (state IN (
        'accepted',
        'running',
        'waiting_for_tool_result',
        'waiting_for_permission',
        'continuing',
        'completed',
        'failed',
        'cancelled'
    )),
    active_tool_call_id TEXT NULL,
    active_permission_id TEXT NULL,
    active_request_id UUID NULL,
    terminal_reason TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_bearwire_runs_one_active_per_session
    ON bearwire_runs(session_id)
    WHERE state IN (
        'accepted',
        'running',
        'waiting_for_tool_result',
        'waiting_for_permission',
        'continuing'
    );

CREATE INDEX IF NOT EXISTS idx_bearwire_runs_bear_created
    ON bearwire_runs (bear_id, created_at DESC);
