-- ADR-0056 v1 Increment 1: claim-before-side-effects durable attempt ledger.
-- Existing Phase 0/1 rows are retained as settled forensic history.

CREATE TABLE docket_turn_claims (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    routing_decision_id UUID NOT NULL UNIQUE REFERENCES docket_routing_decisions (id) ON DELETE CASCADE,
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    job_id UUID NOT NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    work_run_id UUID NULL REFERENCES bear_work_runs (id) ON DELETE SET NULL,
    owner_id TEXT NOT NULL,
    expected_versions JSONB NOT NULL DEFAULT '{}'::jsonb,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    state TEXT NOT NULL DEFAULT 'reserved'
        CHECK (state IN ('reserved', 'executing', 'settled', 'abandoned')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(owner_id) <> '')
);
CREATE INDEX docket_turn_claims_recovery_idx
    ON docket_turn_claims (lease_expires_at) WHERE state IN ('reserved', 'executing');
CREATE UNIQUE INDEX docket_turn_claims_active_task_idx
    ON docket_turn_claims (run_id, task_id) WHERE state IN ('reserved', 'executing');

ALTER TABLE docket_turn_attempts
    ADD COLUMN claim_id UUID NULL REFERENCES docket_turn_claims (id) ON DELETE SET NULL,
    ADD COLUMN observed_boundary TEXT NULL,
    ADD COLUMN normalized_outcome TEXT NULL,
    ADD COLUMN supervisor_disposition TEXT NULL,
    ADD COLUMN recovery_action TEXT NULL,
    ADD COLUMN last_successful_activity JSONB NULL,
    ADD COLUMN failing_boundary JSONB NULL,
    ADD COLUMN criteria_evidence JSONB NULL,
    ADD COLUMN synthetic_provenance JSONB NULL;

-- Remove both legacy checks before rewriting their constrained state values.
-- Legacy `terminal` is preserved as v1 `settled`; open legacy rows are
-- executing because they were already dispatched before this migration.
ALTER TABLE docket_turn_attempts DROP CONSTRAINT IF EXISTS docket_turn_attempts_check;
ALTER TABLE docket_turn_attempts DROP CONSTRAINT IF EXISTS docket_turn_attempts_state_check;
UPDATE docket_turn_attempts
SET state = CASE state WHEN 'terminal' THEN 'settled' ELSE 'executing' END;
ALTER TABLE docket_turn_attempts ADD CONSTRAINT docket_turn_attempts_state_check
    CHECK (state IN ('reserved', 'executing', 'settled', 'abandoned'));
DROP INDEX IF EXISTS docket_turn_attempts_open_idx;
CREATE INDEX docket_turn_attempts_open_idx
    ON docket_turn_attempts (last_activity_at) WHERE state IN ('reserved', 'executing');
ALTER TABLE docket_turn_attempts ADD CONSTRAINT docket_turn_attempts_lifecycle_check CHECK (
    (state IN ('reserved', 'executing') AND outcome IS NULL AND finished_at IS NULL)
    OR (state = 'settled' AND outcome IS NOT NULL AND finished_at IS NOT NULL)
    OR (state = 'abandoned' AND finished_at IS NOT NULL)
);
CREATE UNIQUE INDEX docket_turn_attempts_claim_attempt_idx
    ON docket_turn_attempts (claim_id, attempt) WHERE claim_id IS NOT NULL;

COMMENT ON TABLE docket_turn_claims IS
    'Atomic invocation authority. A current unexpired claim is required before model or tool side effects.';
COMMENT ON COLUMN docket_turn_attempts.normalized_outcome IS
    'Typed normalized outcome projection. The legacy outcome column remains readable during migration.';
COMMENT ON COLUMN docket_turn_attempts.synthetic_provenance IS
    'Synthetic watchdog, provider-disconnect, or process-loss provenance; never fabricated provider output.';
