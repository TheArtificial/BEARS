-- ADR-0056 v1 Increment 2: supervisor-owned disposition and durable delivery.

ALTER TABLE docket_turn_attempts
    ADD CONSTRAINT docket_turn_attempts_supervisor_disposition_check
    CHECK (supervisor_disposition IS NULL OR supervisor_disposition IN (
        'complete', 'retry', 'escalate', 'handoff', 'await_recovery'
    ));

CREATE TABLE docket_notification_outbox (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES bear_job_runs (id) ON DELETE CASCADE,
    task_id UUID NULL REFERENCES bear_tasks (id) ON DELETE CASCADE,
    deduplication_key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('attention', 'retry_scheduled', 'completion')),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    delivered_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(deduplication_key) <> '')
);
CREATE INDEX docket_notification_outbox_pending_idx
    ON docket_notification_outbox (created_at) WHERE delivered_at IS NULL;

COMMENT ON TABLE docket_notification_outbox IS
    'Durable notification intent. Delivery and acknowledgements are owned by channel workers.';
