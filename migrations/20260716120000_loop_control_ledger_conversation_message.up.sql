-- Correlates transcript-free loop-control decisions to the immutable message
-- that caused them, without duplicating transcript content in the ledger.
ALTER TABLE bear_loop_control_ledger
    ADD COLUMN IF NOT EXISTS conversation_message_id UUID NULL
        REFERENCES conversation_messages(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_bear_loop_control_ledger_conversation_message
    ON bear_loop_control_ledger (conversation_message_id, created_at ASC)
    WHERE conversation_message_id IS NOT NULL;

COMMENT ON COLUMN bear_loop_control_ledger.conversation_message_id IS
    'Canonical conversation_messages.id that triggered this controller decision; transcript content remains outside this ledger.';