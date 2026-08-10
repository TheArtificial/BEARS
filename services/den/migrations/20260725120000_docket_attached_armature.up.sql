ALTER TABLE bear_work_runs
    ADD COLUMN execution_target TEXT NOT NULL DEFAULT 'sandbox',
    ADD COLUMN attached_client_session_id TEXT NULL,
    ADD COLUMN attachment_state TEXT NULL,
    ADD COLUMN attachment_warning TEXT NULL,
    ADD COLUMN disconnected_at TIMESTAMPTZ NULL,
    ADD COLUMN disconnect_deadline_at TIMESTAMPTZ NULL;

ALTER TABLE bear_work_runs
    ADD CONSTRAINT bear_work_runs_execution_target_check
    CHECK (execution_target IN ('sandbox', 'attached_armature')),
    ADD CONSTRAINT bear_work_runs_attachment_check CHECK (
        (execution_target = 'sandbox' AND attached_client_session_id IS NULL)
        OR
        (execution_target = 'attached_armature' AND attached_client_session_id IS NOT NULL)
    ),
    ADD CONSTRAINT bear_work_runs_attachment_state_check CHECK (
        attachment_state IS NULL OR attachment_state IN (
            'attached', 'permission_required', 'disconnected', 'timed_out', 'recovered'
        )
    );

CREATE INDEX idx_bear_work_runs_attached_session
    ON bear_work_runs (bear_id, attached_client_session_id)
    WHERE execution_target = 'attached_armature'
      AND state IN ('queued', 'claimed', 'provisioning', 'running', 'paused', 'reporting');
