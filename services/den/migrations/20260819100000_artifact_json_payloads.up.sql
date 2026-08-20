CREATE TABLE IF NOT EXISTS artifact_json_payloads (
    artifact_id UUID PRIMARY KEY REFERENCES artifacts (id) ON DELETE CASCADE,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (jsonb_typeof(payload) IN ('object', 'array')),
    CHECK (octet_length(payload::text) <= 262144)
);
