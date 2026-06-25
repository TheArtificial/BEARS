CREATE TABLE IF NOT EXISTS tool_output_artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    session_id TEXT NOT NULL,
    conversation_id TEXT NULL,
    run_id TEXT NULL,
    tool_call_id TEXT NOT NULL,
    tool_name TEXT NULL,
    source TEXT NOT NULL CHECK (source IN ('den_hosted', 'bearwire_client', 'armature_local', 'mcp')),
    content_text TEXT NULL,
    content_json JSONB NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    content_bytes BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (content_text IS NOT NULL OR content_json IS NOT NULL),
    CHECK (content_json IS NULL OR jsonb_typeof(content_json) IN ('object', 'array', 'string', 'number', 'boolean', 'null')),
    CHECK (jsonb_typeof(metadata) = 'object')
);

CREATE INDEX IF NOT EXISTS idx_tool_output_artifacts_scope
    ON tool_output_artifacts (bear_id, session_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_tool_output_artifacts_tool_call
    ON tool_output_artifacts (bear_id, session_id, tool_call_id, created_at DESC);
