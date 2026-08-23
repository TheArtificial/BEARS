-- Immutable synthetic provenance for canonical attempt recovery. The recovery
-- key makes reconciliation delivery idempotent without reopening authority.
CREATE TABLE docket_execution_attempt_recoveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    execution_attempt_id UUID NOT NULL REFERENCES docket_execution_attempts (id) ON DELETE CASCADE,
    fence_epoch BIGINT NOT NULL CHECK (fence_epoch > 0),
    recovery_key UUID NOT NULL UNIQUE,
    recovery_reason TEXT NOT NULL CHECK (btrim(recovery_reason) <> ''),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (execution_attempt_id, fence_epoch, recovery_key)
);

CREATE INDEX docket_execution_attempt_recoveries_attempt_idx
    ON docket_execution_attempt_recoveries (execution_attempt_id, created_at DESC);
