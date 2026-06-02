DROP INDEX IF EXISTS idx_conversation_messages_source_event_id;

ALTER TABLE conversation_messages
    DROP COLUMN IF EXISTS source_event_id;
