-- Separate stable focused-execution ownership from replaceable host-run evidence.
-- Legacy Pair/Work columns remain compatibility projections during migration.
ALTER TABLE docket_execution_attempts
    ADD COLUMN binding_kind TEXT,
    ADD COLUMN binding_id TEXT,
    ADD COLUMN host_kind TEXT,
    ADD COLUMN host_run_id TEXT;

UPDATE docket_execution_attempts
SET binding_kind = CASE owner_kind
        WHEN 'pair' THEN 'client_session'
        WHEN 'work' THEN 'work_assignment'
    END,
    binding_id = CASE owner_kind
        WHEN 'pair' THEN pair_session_id
        WHEN 'work' THEN work_run_id::text
    END,
    host_kind = owner_kind,
    host_run_id = CASE owner_kind
        WHEN 'pair' THEN pair_run_id::text
        WHEN 'work' THEN work_run_id::text
    END;

ALTER TABLE docket_execution_attempts
    ALTER COLUMN binding_kind SET NOT NULL,
    ALTER COLUMN binding_id SET NOT NULL,
    ALTER COLUMN host_kind SET NOT NULL,
    ALTER COLUMN host_run_id SET NOT NULL,
    ADD CONSTRAINT docket_execution_attempts_binding_kind_check
        CHECK (binding_kind IN ('client_session', 'work_assignment')),
    ADD CONSTRAINT docket_execution_attempts_host_kind_check
        CHECK (host_kind IN ('pair', 'work')),
    ADD CONSTRAINT docket_execution_attempts_binding_id_nonempty
        CHECK (btrim(binding_id) <> ''),
    ADD CONSTRAINT docket_execution_attempts_host_run_id_nonempty
        CHECK (btrim(host_run_id) <> '');

CREATE UNIQUE INDEX docket_execution_attempts_live_binding_idx
    ON docket_execution_attempts (bear_id, binding_kind, binding_id)
    WHERE state IN ('authorized', 'running', 'paused', 'awaiting_user', 'stopping');

CREATE INDEX docket_execution_attempts_host_run_idx
    ON docket_execution_attempts (bear_id, host_kind, host_run_id);

COMMENT ON COLUMN docket_execution_attempts.binding_id IS
    'Stable host-neutral owner identity; authoritative for exclusive focused-execution acquisition.';
COMMENT ON COLUMN docket_execution_attempts.host_run_id IS
    'Replaceable host-run correlation and terminal evidence; not execution authority.';
