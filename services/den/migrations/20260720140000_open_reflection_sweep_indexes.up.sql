-- Keep open-session reflection sweeps from repeatedly scanning unrelated
-- BearWire events and archived/closed client sessions.
CREATE INDEX IF NOT EXISTS idx_bearwire_events_session_identity_type_created
    ON bearwire_events (session_id, bear_id, user_id, event_type, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_client_sessions_open_reflection
    ON client_sessions (updated_at, bear_id, id)
    WHERE closed_at IS NULL AND archived_at IS NULL;
