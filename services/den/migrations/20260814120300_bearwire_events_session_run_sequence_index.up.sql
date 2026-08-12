-- no-transaction
-- A run-state lookup must not scan all high-volume delta events in an ACP session.
-- PostgreSQL cannot create an index concurrently inside a transaction.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_bearwire_events_session_run_sequence
    ON bearwire_events (session_id, (event_json->>'run_id'), sequence_no DESC);
