ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS next_message_sequence BIGINT NOT NULL DEFAULT 0;

ALTER TABLE conversations
    DROP CONSTRAINT IF EXISTS conversations_next_message_sequence_nonnegative;

ALTER TABLE conversations
    ADD CONSTRAINT conversations_next_message_sequence_nonnegative
    CHECK (next_message_sequence >= 0);

UPDATE conversations c
SET next_message_sequence = COALESCE(m.next_sequence, 0)
FROM (
    SELECT conversation_id, MAX(sequence_no) + 1 AS next_sequence
    FROM conversation_messages
    GROUP BY conversation_id
) m
WHERE c.id = m.conversation_id;
