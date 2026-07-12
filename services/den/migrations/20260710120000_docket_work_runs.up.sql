-- Work runs: durable dispatch/claim/lease state for autonomous `work`-stance
-- execution in sandboxes (RUN_SANDBOX provider). One row per dispatch attempt
-- of one task. Lifecycle audit reuses bear_task_events; job linkage reuses
-- bear_job_runs. This is deliberately the entire new durable surface —
-- sandbox-host state (containers, workspaces) lives on the sandbox server and
-- is reconciled by labels, not mirrored here.

CREATE TABLE IF NOT EXISTS bear_work_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    job_id UUID NOT NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    job_run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    attempt INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'claimed', 'provisioning', 'running', 'reporting',
                         'succeeded', 'blocked', 'failed', 'cancelled', 'timed_out')),
    -- Dispatch-worker claim/lease. A run whose lease expired while non-terminal
    -- is reclaimable by any worker.
    runner_id TEXT NULL,
    lease_expires_at TIMESTAMPTZ NULL,
    cancel_requested BOOLEAN NOT NULL DEFAULT FALSE,
    -- Requested provisioning inputs.
    root_name TEXT NULL,
    git_ref TEXT NULL,
    -- Sandbox placement, recorded at provision time.
    sandbox_server_url TEXT NULL,
    sandbox_id TEXT NULL,
    sandbox_type TEXT NULL,
    sandbox_strength TEXT NULL,
    work_surface JSONB NULL,
    -- BearWire session the in-sandbox armature opened for this run.
    bearwire_session_id TEXT NULL,
    result_summary TEXT NULL,
    result_refs JSONB NULL,
    usage JSONB NULL,
    error TEXT NULL,
    queued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ NULL,
    finished_at TIMESTAMPTZ NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (work_surface IS NULL OR jsonb_typeof(work_surface) = 'object'),
    CHECK (result_refs IS NULL OR jsonb_typeof(result_refs) = 'object'),
    CHECK (usage IS NULL OR jsonb_typeof(usage) = 'object')
);

-- No duplicate concurrent dispatch of one task.
CREATE UNIQUE INDEX IF NOT EXISTS idx_bear_work_runs_one_active_per_task
    ON bear_work_runs (task_id)
    WHERE state IN ('queued', 'claimed', 'provisioning', 'running', 'reporting');

CREATE INDEX IF NOT EXISTS idx_bear_work_runs_claimable
    ON bear_work_runs (state, lease_expires_at);

CREATE INDEX IF NOT EXISTS idx_bear_work_runs_session
    ON bear_work_runs (bearwire_session_id)
    WHERE bearwire_session_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_bear_work_runs_job_time
    ON bear_work_runs (job_id, queued_at DESC);

CREATE INDEX IF NOT EXISTS idx_bear_work_runs_bear_time
    ON bear_work_runs (bear_id, queued_at DESC);
