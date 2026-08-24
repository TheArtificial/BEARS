CREATE TABLE runtime_exception_events (
    id UUID PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('warning', 'error')),
    component TEXT NOT NULL,
    event_code TEXT NOT NULL,
    message TEXT NOT NULL,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    session_id TEXT,
    runtime_run_id TEXT,
    work_run_id UUID,
    docket_job_id UUID,
    docket_task_id UUID,
    conversation_id TEXT,
    bear_id UUID,
    build_revision TEXT
);

CREATE INDEX runtime_exception_events_recent_idx
    ON runtime_exception_events (created_at DESC);
CREATE INDEX runtime_exception_events_bear_recent_idx
    ON runtime_exception_events (bear_id, created_at DESC)
    WHERE bear_id IS NOT NULL;
CREATE INDEX runtime_exception_events_work_run_idx
    ON runtime_exception_events (work_run_id, created_at DESC)
    WHERE work_run_id IS NOT NULL;
CREATE INDEX runtime_exception_events_runtime_run_idx
    ON runtime_exception_events (runtime_run_id, created_at DESC)
    WHERE runtime_run_id IS NOT NULL;
CREATE INDEX runtime_exception_events_code_idx
    ON runtime_exception_events (event_code, created_at DESC);
