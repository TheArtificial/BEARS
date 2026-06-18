ALTER TABLE conversations
    DROP CONSTRAINT IF EXISTS conversations_next_message_sequence_nonnegative;

ALTER TABLE conversations
    DROP COLUMN IF EXISTS next_message_sequence;
