-- ADR-0056 Phase 0/1: routing metadata, attention cursors, durable placement,
-- attempt outcomes, and conversation bindings.

ALTER TABLE bear_tasks
    ADD COLUMN routing_strategy TEXT NOT NULL DEFAULT 'auto'
        CHECK (routing_strategy IN ('inline', 'scoped', 'delegated', 'auto')),
    ADD COLUMN expected_context_size INTEGER NULL
        CHECK (expected_context_size IS NULL OR expected_context_size >= 0),
    ADD COLUMN result_rollup_policy TEXT NULL
        CHECK (result_rollup_policy IS NULL OR result_rollup_policy IN ('summary_to_parent', 'none'));

ALTER TABLE bear_work_runs DROP CONSTRAINT IF EXISTS bear_work_runs_state_check;
ALTER TABLE bear_work_runs ADD CONSTRAINT bear_work_runs_state_check CHECK (
    state IN ('queued', 'claimed', 'provisioning', 'running', 'paused', 'reporting',
              'succeeded', 'blocked', 'failed', 'cancelled', 'timed_out')
);

CREATE TABLE docket_conversation_bindings (
    task_id UUID PRIMARY KEY REFERENCES bear_tasks (id) ON DELETE CASCADE,
    preferred_conversation_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(preferred_conversation_id) <> '')
);

CREATE TABLE docket_conversation_binding_runs (
    run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, task_id),
    CHECK (btrim(conversation_id) <> '')
);

-- Existing execution rows contain the durable task/conversation association.
-- Keep the newest association as preferred while retaining each run's history.
INSERT INTO docket_conversation_bindings (task_id, preferred_conversation_id, created_at, updated_at)
SELECT DISTINCT ON (task_id)
    task_id, source_conversation_id, created_at, updated_at
FROM docket_execution_sessions
WHERE task_id IS NOT NULL AND source_conversation_id IS NOT NULL
ORDER BY task_id, updated_at DESC, id DESC
ON CONFLICT (task_id) DO NOTHING;

INSERT INTO docket_conversation_binding_runs (run_id, task_id, conversation_id, created_at)
SELECT DISTINCT ON (run_id, task_id)
    run_id, task_id, source_conversation_id, created_at
FROM docket_execution_sessions
WHERE task_id IS NOT NULL AND source_conversation_id IS NOT NULL
ORDER BY run_id, task_id, updated_at DESC, id DESC
ON CONFLICT (run_id, task_id) DO NOTHING;

CREATE TABLE docket_cursors (
    client_session_id TEXT PRIMARY KEY,
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    job_id UUID NOT NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    task_id UUID NULL REFERENCES bear_tasks (id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(client_session_id) <> '')
);
CREATE INDEX docket_cursors_job_idx ON docket_cursors (job_id, updated_at DESC);

CREATE TABLE docket_routing_decisions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    idempotency_key UUID NOT NULL UNIQUE,
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    job_id UUID NOT NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    turn_source TEXT NOT NULL CHECK (turn_source IN ('user', 'continuation', 'dispatch', 'rollup')),
    conversation_strategy TEXT NOT NULL CHECK (conversation_strategy IN ('reuse', 'inline', 'scoped', 'delegated')),
    conversation_id TEXT NOT NULL,
    parent_conversation_id TEXT NULL,
    routing_strategy TEXT NOT NULL CHECK (routing_strategy IN ('inline', 'scoped', 'delegated', 'auto')),
    execution_surface TEXT NOT NULL CHECK (execution_surface IN ('sandbox', 'armature')),
    resolved_profile TEXT NULL,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt > 0),
    cursor_before JSONB NULL,
    cursor_after JSONB NULL,
    reason TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(conversation_id) <> ''),
    CHECK (btrim(reason) <> '')
);
CREATE INDEX docket_routing_decisions_job_idx
    ON docket_routing_decisions (job_id, run_id, created_at, id);

CREATE TABLE docket_turn_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    routing_decision_id UUID NOT NULL REFERENCES docket_routing_decisions (id) ON DELETE CASCADE,
    work_run_id UUID NULL REFERENCES bear_work_runs (id) ON DELETE SET NULL,
    attempt INTEGER NOT NULL CHECK (attempt > 0),
    state TEXT NOT NULL DEFAULT 'running' CHECK (state IN ('running', 'terminal')),
    outcome TEXT NULL CHECK (outcome IS NULL OR outcome IN ('completed', 'blocked', 'failed', 'timed_out', 'cancelled')),
    cause_code TEXT NULL,
    retry_disposition TEXT NULL CHECK (retry_disposition IS NULL OR retry_disposition IN ('none', 'retry', 'escalate', 'handoff', 'pause')),
    evidence_refs JSONB NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_activity_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ NULL,
    UNIQUE (routing_decision_id, attempt),
    CHECK ((state = 'running' AND outcome IS NULL AND finished_at IS NULL)
        OR (state = 'terminal' AND outcome IS NOT NULL AND finished_at IS NOT NULL))
);
CREATE INDEX docket_turn_attempts_open_idx
    ON docket_turn_attempts (last_activity_at) WHERE state = 'running';

CREATE UNIQUE INDEX bear_task_run_state_one_in_progress_per_run
    ON bear_task_run_state (run_id) WHERE status = 'in_progress';

CREATE TABLE docket_attention (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    task_id UUID NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    cause_code TEXT NOT NULL,
    recovery_action TEXT NOT NULL,
    evidence_refs JSONB NOT NULL DEFAULT '{}'::jsonb,
    resolved_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX docket_attention_one_open_per_run
    ON docket_attention (run_id) WHERE resolved_at IS NULL;

CREATE TABLE docket_result_rollups (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    parent_task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    summary TEXT NOT NULL CHECK (btrim(summary) <> ''),
    evidence_refs JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, task_id)
);
CREATE INDEX docket_result_rollups_parent_idx
    ON docket_result_rollups (run_id, parent_task_id, created_at, task_id);

COMMENT ON TABLE docket_cursors IS 'Per-client attention only. Cursors never select or claim executable work.';
COMMENT ON TABLE docket_conversation_bindings IS 'Preferred durable transcript container for a task; execution position remains run state.';
COMMENT ON TABLE docket_routing_decisions IS 'Immutable ADR-0056 placement decisions for every routed turn.';
COMMENT ON TABLE docket_turn_attempts IS 'Durable normalized outcome envelope; replayable activity remains in the canonical event stream.';
