-- Pre-dispatch scheduler rejections are Docket control-plane evidence, not
-- executor retry state. One active streak per durable Work execution binding
-- and rejection reason is enough for the initial intervention policy.
CREATE TABLE bear_execution_rejection_observations (
    work_run_id UUID NOT NULL REFERENCES bear_work_runs (id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    occurrences INTEGER NOT NULL DEFAULT 1 CHECK (occurrences > 0),
    last_observed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (work_run_id, reason)
);
