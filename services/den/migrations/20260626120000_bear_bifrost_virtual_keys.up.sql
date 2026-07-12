CREATE TABLE IF NOT EXISTS bear_bifrost_virtual_keys (
    bear_id UUID PRIMARY KEY REFERENCES bears(id) ON DELETE CASCADE,
    virtual_key_id TEXT NULL,
    virtual_key_name TEXT NULL,
    virtual_key_value TEXT NULL,
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
