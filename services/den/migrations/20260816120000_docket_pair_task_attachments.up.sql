-- Durable Docket ownership and a temporary Pair-session claim are independent.
CREATE TABLE bear_pair_task_attachments (
    task_id UUID PRIMARY KEY REFERENCES bear_tasks (id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES client_sessions (id) ON DELETE CASCADE,
    attached_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_at TIMESTAMPTZ NULL
);

CREATE INDEX idx_bear_pair_task_attachments_active_session
    ON bear_pair_task_attachments (session_id, attached_at)
    WHERE released_at IS NULL;
