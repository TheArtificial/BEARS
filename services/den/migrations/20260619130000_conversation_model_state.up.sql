CREATE TABLE IF NOT EXISTS conversation_model_state (
    conversation_id UUID PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    selection_mode TEXT NOT NULL DEFAULT 'auto' CHECK (selection_mode IN ('auto', 'explicit')),
    requested_model TEXT NULL,
    selected_model TEXT NULL,
    selected_reason TEXT NULL,
    actual_last_model TEXT NULL,
    actual_last_provider TEXT NULL,
    fallback_count INTEGER NOT NULL DEFAULT 0,
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_conversation_model_state_updated
    ON conversation_model_state (updated_at DESC);
