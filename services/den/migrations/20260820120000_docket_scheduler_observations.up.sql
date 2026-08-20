CREATE TABLE docket_scheduler_observations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    execution_session_id UUID NOT NULL REFERENCES docket_execution_sessions(id) ON DELETE CASCADE,
    job_id UUID NOT NULL REFERENCES bear_jobs(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES bear_job_runs(id) ON DELETE CASCADE,
    task_id UUID REFERENCES bear_tasks(id) ON DELETE SET NULL,
    reason TEXT NOT NULL,
    occurrence INTEGER NOT NULL CHECK (occurrence > 0),
    disposition TEXT NOT NULL CHECK (disposition IN ('reconcile', 'stop')),
    delivery_state TEXT NOT NULL DEFAULT 'pending' CHECK (delivery_state IN ('pending', 'delivered')),
    delivered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (execution_session_id, reason, occurrence)
);

CREATE INDEX docket_scheduler_observations_pending_session_idx
    ON docket_scheduler_observations (execution_session_id, created_at)
    WHERE delivery_state = 'pending';
