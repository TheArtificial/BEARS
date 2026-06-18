CREATE TABLE IF NOT EXISTS bearwire_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sequence_no BIGSERIAL NOT NULL,
    session_id TEXT NOT NULL,
    bear_id UUID NULL REFERENCES bears(id) ON DELETE CASCADE,
    user_id INTEGER NULL REFERENCES users(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    event_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_bearwire_events_session_sequence
    ON bearwire_events (session_id, sequence_no);

CREATE INDEX IF NOT EXISTS idx_bearwire_events_bear_created
    ON bearwire_events (bear_id, created_at DESC);
