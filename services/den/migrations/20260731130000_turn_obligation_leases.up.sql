-- ADR-0048: fenced renewable ownership for armature-local tool execution.
ALTER TABLE turn_obligations
    ADD COLUMN IF NOT EXISTS lease_attempt_token_hash TEXT NULL,
    ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ NULL,
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ NULL;

ALTER TABLE turn_obligations
    DROP CONSTRAINT IF EXISTS turn_obligations_lease_shape_check;

ALTER TABLE turn_obligations
    ADD CONSTRAINT turn_obligations_lease_shape_check CHECK (
        (lease_attempt_token_hash IS NULL AND claimed_at IS NULL AND lease_expires_at IS NULL)
        OR
        (kind = 'tool_result'
         AND lease_attempt_token_hash IS NOT NULL
         AND claimed_at IS NOT NULL
         AND lease_expires_at IS NOT NULL)
    );

CREATE INDEX IF NOT EXISTS idx_turn_obligations_open_lease_expiry
    ON turn_obligations (lease_expires_at)
    WHERE state = 'waiting_for_client' AND lease_expires_at IS NOT NULL;
