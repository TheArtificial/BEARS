CREATE TABLE IF NOT EXISTS bearwire_run_obligations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id TEXT NOT NULL REFERENCES bearwire_runs(run_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('tool_call', 'permission')),
    expected_client_method TEXT NOT NULL CHECK (
        expected_client_method IN ('client.tool.result', 'client.permission.result')
    ),
    tool_call_id TEXT NULL,
    permission_id TEXT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'requested',
        'waiting_for_client',
        'result_received',
        'continued',
        'failed',
        'cancelled'
    )),
    request_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    result_payload JSONB NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ NULL,
    CHECK (tool_call_id IS NOT NULL OR permission_id IS NOT NULL)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_bearwire_obligations_tool_call
    ON bearwire_run_obligations(run_id, tool_call_id)
    WHERE tool_call_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_bearwire_obligations_permission
    ON bearwire_run_obligations(run_id, permission_id)
    WHERE permission_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_bearwire_obligations_run_state
    ON bearwire_run_obligations(run_id, state, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_bearwire_obligations_session_state
    ON bearwire_run_obligations(session_id, state, updated_at DESC);
