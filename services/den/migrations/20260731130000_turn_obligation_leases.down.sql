DROP INDEX IF EXISTS idx_turn_obligations_open_lease_expiry;

ALTER TABLE turn_obligations
    DROP CONSTRAINT IF EXISTS turn_obligations_lease_shape_check,
    DROP COLUMN IF EXISTS lease_expires_at,
    DROP COLUMN IF EXISTS claimed_at,
    DROP COLUMN IF EXISTS lease_attempt_token_hash;
