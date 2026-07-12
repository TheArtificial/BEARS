ALTER TABLE turn_runs
    DROP CONSTRAINT IF EXISTS turn_runs_state_check;

ALTER TABLE turn_runs
    ADD CONSTRAINT turn_runs_state_check CHECK (state IN (
        'accepted',
        'running',
        'waiting_for_client',
        'waiting_for_tool_result',
        'waiting_for_permission',
        'continuing',
        'completed',
        'failed',
        'cancelled'
    ));

DROP INDEX IF EXISTS idx_turn_runs_one_active_per_session;
CREATE UNIQUE INDEX idx_turn_runs_one_active_per_session
    ON turn_runs (session_id)
    WHERE state IN (
        'accepted',
        'running',
        'waiting_for_client',
        'waiting_for_tool_result',
        'waiting_for_permission',
        'continuing'
    );
