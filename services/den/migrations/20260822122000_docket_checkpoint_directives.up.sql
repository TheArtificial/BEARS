-- A Work attempt must checkpoint before Docket will consider a fresh dispatch.
CREATE TABLE docket_checkpoint_directives (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    execution_attempt_id UUID NOT NULL REFERENCES docket_execution_attempts (id) ON DELETE CASCADE,
    fence_epoch BIGINT NOT NULL CHECK (fence_epoch > 0),
    state TEXT NOT NULL CHECK (state IN ('pending', 'acknowledged', 'superseded')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    acknowledged_at TIMESTAMPTZ NULL,
    superseded_at TIMESTAMPTZ NULL,
    CHECK (state <> 'acknowledged' OR acknowledged_at IS NOT NULL),
    CHECK (state <> 'superseded' OR superseded_at IS NOT NULL)
);

CREATE UNIQUE INDEX docket_checkpoint_directives_attempt_fence_idx
    ON docket_checkpoint_directives (execution_attempt_id, fence_epoch);

COMMENT ON TABLE docket_checkpoint_directives IS
    'Docket-owned checkpoint requirements bound to one canonical Work attempt fence.';
