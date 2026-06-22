-- Docket relational jobs/tasks model (ADR-0034 / ADR-0045).
-- This is additive alongside the legacy bear_work_plans activity board. The
-- legacy board remains the session task-list/activity projection until the
-- follow-up migration bridges and retires it.

CREATE TABLE IF NOT EXISTS bear_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    created_by_user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_by_role TEXT NOT NULL CHECK (created_by_role IN ('chat', 'pair', 'ui')),
    goal TEXT NOT NULL,
    work_surface_ref TEXT NULL,
    commit_policy TEXT NULL CHECK (commit_policy IS NULL OR commit_policy IN ('none', 'per_task', 'per_job', 'propose_only')),
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'ready', 'running', 'blocked', 'completed', 'cancelled')),
    visibility TEXT NOT NULL DEFAULT 'private_to_profile'
        CHECK (visibility IN ('private_to_profile', 'same_user', 'bear_visible', 'handoff_requested')),
    current_run_id UUID NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(goal) <> '')
);

CREATE INDEX IF NOT EXISTS idx_bear_jobs_bear_status
    ON bear_jobs (bear_id, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_bear_jobs_created_by_user
    ON bear_jobs (created_by_user_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_bear_jobs_work_surface
    ON bear_jobs (bear_id, work_surface_ref, updated_at DESC)
    WHERE work_surface_ref IS NOT NULL;

CREATE TABLE IF NOT EXISTS bear_job_criteria (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id UUID NOT NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('narrative', 'command', 'check_ref')),
    description TEXT NOT NULL,
    spec JSONB NULL,
    sibling_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(description) <> ''),
    CHECK (spec IS NULL OR jsonb_typeof(spec) = 'object')
);

CREATE INDEX IF NOT EXISTS idx_bear_job_criteria_job_order
    ON bear_job_criteria (job_id, sibling_order, created_at);

CREATE TABLE IF NOT EXISTS bear_tasks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    job_id UUID NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    session_anchor_id UUID NULL REFERENCES acp_sessions (id) ON DELETE SET NULL,
    parent_task_id UUID NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    sibling_order INTEGER NOT NULL DEFAULT 0,
    kind TEXT NOT NULL DEFAULT 'execution'
        CHECK (kind IN ('execution', 'investigation', 'decision')),
    scope TEXT NOT NULL DEFAULT 'template'
        CHECK (scope IN ('template', 'run')),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    difficulty TEXT NULL CHECK (difficulty IS NULL OR difficulty IN ('trivial', 'moderate', 'hard', 'unknown')),
    effort_hint TEXT NULL CHECK (effort_hint IS NULL OR effort_hint IN ('low', 'medium', 'high')),
    assigned_to_role TEXT NULL CHECK (assigned_to_role IS NULL OR assigned_to_role IN ('chat', 'pair', 'curate', 'work', 'watch')),
    created_by_role TEXT NOT NULL CHECK (created_by_role IN ('chat', 'pair', 'curate', 'work', 'watch', 'system', 'ui')),
    created_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    created_by_agent_id TEXT NULL,
    created_in_run_id UUID NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (job_id IS NOT NULL OR session_anchor_id IS NOT NULL),
    CHECK (btrim(title) <> ''),
    CHECK (btrim(body) <> '')
);

CREATE INDEX IF NOT EXISTS idx_bear_tasks_job_parent_order
    ON bear_tasks (job_id, parent_task_id, sibling_order, created_at)
    WHERE job_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_bear_tasks_session_anchor
    ON bear_tasks (session_anchor_id, parent_task_id, sibling_order, created_at)
    WHERE session_anchor_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_bear_tasks_parent
    ON bear_tasks (parent_task_id, sibling_order, created_at)
    WHERE parent_task_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS bear_job_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id UUID NOT NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    trigger TEXT NOT NULL DEFAULT 'manual'
        CHECK (trigger IN ('manual', 'scheduled', 'event')),
    schedule_ref TEXT NULL,
    state TEXT NOT NULL DEFAULT 'dispatched'
        CHECK (state IN ('dispatched', 'running', 'paused', 'completed', 'failed', 'cancelled')),
    started_at TIMESTAMPTZ NULL,
    finished_at TIMESTAMPTZ NULL,
    outcome JSONB NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (outcome IS NULL OR jsonb_typeof(outcome) = 'object')
);

CREATE INDEX IF NOT EXISTS idx_bear_job_runs_job_time
    ON bear_job_runs (job_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_bear_job_runs_state
    ON bear_job_runs (state, updated_at DESC);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'bear_jobs_current_run_fk'
    ) THEN
        ALTER TABLE bear_jobs
            ADD CONSTRAINT bear_jobs_current_run_fk
            FOREIGN KEY (current_run_id) REFERENCES bear_job_runs (id) ON DELETE SET NULL;
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'bear_tasks_created_in_run_fk'
    ) THEN
        ALTER TABLE bear_tasks
            ADD CONSTRAINT bear_tasks_created_in_run_fk
            FOREIGN KEY (created_in_run_id) REFERENCES bear_job_runs (id) ON DELETE SET NULL;
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS bear_task_run_state (
    run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'in_progress', 'done', 'blocked', 'cancelled')),
    result_refs JSONB NULL,
    result_summary TEXT NULL,
    started_at TIMESTAMPTZ NULL,
    finished_at TIMESTAMPTZ NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, task_id),
    CHECK (result_refs IS NULL OR jsonb_typeof(result_refs) = 'object')
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_bear_task_run_state_one_in_progress
    ON bear_task_run_state (run_id)
    WHERE status = 'in_progress';

CREATE INDEX IF NOT EXISTS idx_bear_task_run_state_task
    ON bear_task_run_state (task_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS bear_job_criteria_state (
    run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    criterion_id UUID NOT NULL REFERENCES bear_job_criteria (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'unmet'
        CHECK (status IN ('unmet', 'met', 'waived')),
    evaluated_at TIMESTAMPTZ NULL,
    evidence JSONB NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, criterion_id),
    CHECK (evidence IS NULL OR jsonb_typeof(evidence) = 'object')
);

CREATE TABLE IF NOT EXISTS bear_task_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    run_id UUID NULL REFERENCES bear_job_runs (id) ON DELETE SET NULL,
    event_type TEXT NOT NULL CHECK (event_type IN (
        'created',
        'claimed',
        'started',
        'progressed',
        'blocked',
        'completed',
        'cancelled',
        'child_added',
        'updated',
        'reordered'
    )),
    by_role TEXT NOT NULL CHECK (by_role IN ('chat', 'pair', 'curate', 'work', 'watch', 'system', 'ui')),
    by_agent_id TEXT NULL,
    by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (payload IS NULL OR jsonb_typeof(payload) = 'object')
);

CREATE INDEX IF NOT EXISTS idx_bear_task_events_task_time
    ON bear_task_events (task_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_bear_task_events_run_time
    ON bear_task_events (run_id, created_at DESC)
    WHERE run_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS bear_job_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id UUID NOT NULL REFERENCES bear_jobs (id) ON DELETE CASCADE,
    run_id UUID NULL REFERENCES bear_job_runs (id) ON DELETE SET NULL,
    event_type TEXT NOT NULL CHECK (event_type IN (
        'job_created',
        'task_added',
        'task_updated',
        'criterion_evaluated',
        'job_blocked',
        'job_completed',
        'job_cancelled',
        'handoff_requested',
        'note_added',
        'run_started',
        'run_finished'
    )),
    task_id UUID NULL REFERENCES bear_tasks (id) ON DELETE SET NULL,
    by_role TEXT NOT NULL CHECK (by_role IN ('chat', 'pair', 'curate', 'work', 'watch', 'system', 'ui')),
    by_agent_id TEXT NULL,
    by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (payload IS NULL OR jsonb_typeof(payload) = 'object')
);

CREATE INDEX IF NOT EXISTS idx_bear_job_events_job_time
    ON bear_job_events (job_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_bear_job_events_run_time
    ON bear_job_events (run_id, created_at DESC)
    WHERE run_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_bear_job_events_task_time
    ON bear_job_events (task_id, created_at DESC)
    WHERE task_id IS NOT NULL;

COMMENT ON TABLE bear_jobs IS 'Docket-canonical durable job containers: goal, policy, visibility, and current run pointer.';
COMMENT ON TABLE bear_tasks IS 'Docket-canonical durable task definitions. Status/results are run-scoped in bear_task_run_state.';
COMMENT ON TABLE bear_job_runs IS 'Docket job execution attempts. A oneshot job has one run; recurrence creates additional runs.';
COMMENT ON TABLE bear_task_run_state IS 'Run-scoped task execution state; enforces one in_progress task per run.';
COMMENT ON TABLE bear_job_criteria IS 'Docket job acceptance criteria injected into task dispatch and evaluated per run.';
COMMENT ON TABLE bear_job_criteria_state IS 'Run-scoped acceptance-criteria evaluation state.';
COMMENT ON TABLE bear_task_events IS 'Append-only task history for definitions and run-scoped execution events.';
COMMENT ON TABLE bear_job_events IS 'Append-only job report and audit stream.';
