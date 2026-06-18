CREATE TABLE IF NOT EXISTS bearwire_client_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id TEXT NOT NULL REFERENCES bearwire_runs(run_id) ON DELETE CASCADE,
    obligation_kind TEXT NOT NULL CHECK (obligation_kind IN ('tool', 'permission')),
    obligation_id TEXT NOT NULL,
    result_hash TEXT NOT NULL,
    payload_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(run_id, obligation_kind, obligation_id)
);

CREATE INDEX IF NOT EXISTS idx_bearwire_client_results_created
    ON bearwire_client_results (created_at DESC);
