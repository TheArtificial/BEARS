-- A recovery snapshot exists only while a Pair run is durably claimed at a
-- technical budget boundary.  It is deliberately separate from `turn_runs`:
-- ordinary `continuing` also represents client-result continuation and is not
-- eligible for process-loss recovery.
CREATE TABLE turn_run_recovery_snapshots (
    run_id TEXT PRIMARY KEY REFERENCES turn_runs(run_id) ON DELETE CASCADE,
    reason TEXT NOT NULL CHECK (reason IN (
        'wall_clock_limit',
        'total_tool_call_limit',
        'tool_class_call_limit',
        'emergency_hard_step_limit'
    )),
    snapshot JSONB NOT NULL,
    recovery_lease_id UUID NULL,
    recovery_lease_expires_at TIMESTAMPTZ NULL,
    recovered_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK ((recovery_lease_id IS NULL) = (recovery_lease_expires_at IS NULL))
);

CREATE INDEX idx_turn_run_recovery_snapshots_pending
    ON turn_run_recovery_snapshots (created_at)
    WHERE recovered_at IS NULL;
