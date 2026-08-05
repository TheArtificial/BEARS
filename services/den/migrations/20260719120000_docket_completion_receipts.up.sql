CREATE TABLE docket_task_completion_receipts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id UUID NOT NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    run_id UUID NOT NULL,
    primary_output_ref TEXT NOT NULL,
    immutable_identity TEXT NOT NULL,
    validation JSONB NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(primary_output_ref) <> ''),
    CHECK (btrim(immutable_identity) <> ''),
    CHECK (jsonb_typeof(validation) = 'object'),
    UNIQUE (task_id, run_id)
);

CREATE INDEX idx_docket_task_completion_receipts_run
    ON docket_task_completion_receipts (run_id, recorded_at DESC);
