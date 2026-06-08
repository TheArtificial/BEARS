CREATE TABLE IF NOT EXISTS native_runtime_approvals (
    approval_id TEXT PRIMARY KEY,
    bear_id UUID NOT NULL,
    conversation_id TEXT NOT NULL,
    acp_session_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    arguments_json JSONB NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'denied', 'expired')),
    decision_reason TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_at TIMESTAMPTZ NULL,
    expires_at TIMESTAMPTZ NULL
);
CREATE INDEX IF NOT EXISTS idx_native_runtime_approvals_pending
    ON native_runtime_approvals (bear_id, status, created_at);
