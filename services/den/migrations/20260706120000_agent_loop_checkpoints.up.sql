-- ADR-0050: structured agent-loop checkpoint artifacts.
-- These are run audit artifacts, not Docket task/job events and not canonical conversation history.

CREATE TABLE IF NOT EXISTS bear_run_checkpoints (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id TEXT NOT NULL REFERENCES turn_runs(run_id) ON DELETE CASCADE,
    turn_step_id UUID NULL REFERENCES turn_steps(id) ON DELETE SET NULL,
    checkpoint_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    control_level TEXT NOT NULL,
    request JSONB NOT NULL,
    response JSONB NULL,
    validation_status TEXT NOT NULL DEFAULT 'requested' CHECK (validation_status IN (
        'requested',
        'valid',
        'invalid',
        'superseded'
    )),
    visibility TEXT NOT NULL DEFAULT 'audit_only' CHECK (visibility IN (
        'audit_only',
        'live_ephemeral',
        'model_visible_hidden'
    )),
    replay_policy TEXT NOT NULL DEFAULT 'none' CHECK (replay_policy IN (
        'none',
        'summary_once',
        'until_superseded'
    )),
    related_task_list_id TEXT NULL,
    related_task_item_id TEXT NULL,
    related_docket_task_id UUID NULL REFERENCES bear_tasks(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, checkpoint_id)
);

CREATE INDEX IF NOT EXISTS idx_bear_run_checkpoints_run_created
    ON bear_run_checkpoints (run_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_bear_run_checkpoints_docket_task
    ON bear_run_checkpoints (related_docket_task_id, created_at DESC)
    WHERE related_docket_task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_bear_run_checkpoints_task_list
    ON bear_run_checkpoints (related_task_list_id, related_task_item_id, created_at DESC)
    WHERE related_task_list_id IS NOT NULL;

COMMENT ON TABLE bear_run_checkpoints IS 'Structured runtime checkpoint artifacts for agent-loop audit. Not Docket task/job events and not canonical conversation history.';
COMMENT ON COLUMN bear_run_checkpoints.replay_policy IS 'Explicit model-replay policy for the checkpoint artifact. Defaults to none.';
