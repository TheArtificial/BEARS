-- ADR-0048: introduce explicit model-step identity for core turn obligations.
-- This is additive and compatibility-safe: existing run-level barrier code continues to work
-- while new runs can start populating turn_step_id in follow-up phases.

CREATE TABLE IF NOT EXISTS turn_steps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id TEXT NOT NULL REFERENCES bearwire_runs(run_id) ON DELETE CASCADE,
    step_index INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'streaming_model' CHECK (state IN (
        'streaming_model',
        'waiting_for_client',
        'ready_to_continue',
        'continued',
        'failed',
        'cancelled'
    )),
    provider_response_id TEXT NULL,
    opened_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMPTZ NULL,
    UNIQUE (run_id, step_index)
);

ALTER TABLE bearwire_run_obligations
    ADD COLUMN IF NOT EXISTS turn_step_id UUID NULL REFERENCES turn_steps(id) ON DELETE SET NULL;

ALTER TABLE bearwire_client_results
    ADD COLUMN IF NOT EXISTS turn_step_id UUID NULL REFERENCES turn_steps(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_turn_steps_run_state
    ON turn_steps (run_id, state, step_index DESC);

CREATE INDEX IF NOT EXISTS idx_bearwire_obligations_turn_step_state
    ON bearwire_run_obligations (turn_step_id, state, updated_at DESC)
    WHERE turn_step_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_bearwire_client_results_turn_step
    ON bearwire_client_results (turn_step_id, created_at DESC)
    WHERE turn_step_id IS NOT NULL;

COMMENT ON TABLE turn_steps IS 'Model-step identity for core turn coordination. Used to fence obligations and continue the model only after a step barrier closes.';
COMMENT ON COLUMN bearwire_run_obligations.turn_step_id IS 'Optional model-step fence for this turn obligation. Nullable during ADR-0048 migration.';
COMMENT ON COLUMN bearwire_client_results.turn_step_id IS 'Optional model-step fence for this obligation result. Nullable during ADR-0048 migration.';
