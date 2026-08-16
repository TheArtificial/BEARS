ALTER TABLE turn_runs
    DROP CONSTRAINT IF EXISTS turn_runs_state_check;

ALTER TABLE turn_runs
    ADD CONSTRAINT turn_runs_state_check CHECK (state IN (
        'accepted',
        'running',
        'waiting_for_client',
        'continuing',
        'blocked',
        'completed',
        'failed',
        'cancelled'
    ));
