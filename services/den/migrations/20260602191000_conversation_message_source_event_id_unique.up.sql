ALTER TABLE conversation_messages
    ADD COLUMN IF NOT EXISTS source_event_id TEXT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_conversation_messages_source_event_id
    ON conversation_messages (conversation_id, source_event_id)
    WHERE source_event_id IS NOT NULL;
