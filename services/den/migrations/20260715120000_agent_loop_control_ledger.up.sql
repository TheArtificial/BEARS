-- ADR-0050 companion: replayable, transcript-free loop-control decision ledger.
-- Stores typed decision metadata for offline replay/tuning. This is not canonical
-- conversation history and intentionally avoids raw transcript content.

CREATE TABLE IF NOT EXISTS bear_loop_control_ledger (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id TEXT NOT NULL REFERENCES turn_runs(run_id) ON DELETE CASCADE,
    turn_step_id UUID NULL REFERENCES turn_steps(id) ON DELETE SET NULL,
    decision_id TEXT NOT NULL,
    decision_kind TEXT NOT NULL,
    control_level TEXT NOT NULL,
    reason TEXT NULL,
    orientation_kind TEXT NULL,
    checkpoint_id TEXT NULL,
    related_task_list_id TEXT NULL,
    related_task_item_id TEXT NULL,
    related_docket_job_id UUID NULL REFERENCES bear_jobs(id) ON DELETE SET NULL,
    related_docket_task_id UUID NULL REFERENCES bear_tasks(id) ON DELETE SET NULL,
    evidence_refs JSONB NOT NULL DEFAULT '[]'::jsonb,
    decision JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, decision_id)
);

CREATE INDEX IF NOT EXISTS idx_bear_loop_control_ledger_run_created
    ON bear_loop_control_ledger (run_id, created_at ASC, decision_id ASC);

CREATE INDEX IF NOT EXISTS idx_bear_loop_control_ledger_kind_created
    ON bear_loop_control_ledger (decision_kind, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_bear_loop_control_ledger_docket_task
    ON bear_loop_control_ledger (related_docket_task_id, created_at DESC)
    WHERE related_docket_task_id IS NOT NULL;

COMMENT ON TABLE bear_loop_control_ledger IS 'Replayable transcript-free loop-control decision ledger for ADR-0050 tuning.';
COMMENT ON COLUMN bear_loop_control_ledger.evidence_refs IS 'Typed evidence references only; summaries/raw transcript content are intentionally omitted.';
COMMENT ON COLUMN bear_loop_control_ledger.decision IS 'Typed decision payload for offline replay without model calls.';
