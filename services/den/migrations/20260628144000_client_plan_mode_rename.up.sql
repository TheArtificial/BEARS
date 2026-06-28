-- ADR-0043: plan mode is keyed by Den client sessions, not ACP sessions.

DO $$
BEGIN
    IF to_regclass('public.client_plan_mode_sessions') IS NULL
       AND to_regclass('public.acp_plan_mode_sessions') IS NOT NULL THEN
        ALTER TABLE acp_plan_mode_sessions RENAME TO client_plan_mode_sessions;
    END IF;

    IF to_regclass('public.client_plan_mode_events') IS NULL
       AND to_regclass('public.acp_plan_mode_events') IS NOT NULL THEN
        ALTER TABLE acp_plan_mode_events RENAME TO client_plan_mode_events;
    END IF;
END $$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'client_plan_mode_sessions'
          AND column_name = 'acp_session_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'client_plan_mode_sessions'
          AND column_name = 'client_session_id'
    ) THEN
        ALTER TABLE client_plan_mode_sessions RENAME COLUMN acp_session_id TO client_session_id;
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'client_plan_mode_events'
          AND column_name = 'acp_session_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'client_plan_mode_events'
          AND column_name = 'client_session_id'
    ) THEN
        ALTER TABLE client_plan_mode_events RENAME COLUMN acp_session_id TO client_session_id;
    END IF;
END $$;

DROP INDEX IF EXISTS idx_acp_plan_mode_one_open_session;
DROP INDEX IF EXISTS idx_acp_plan_mode_bear_session;
DROP INDEX IF EXISTS idx_acp_plan_mode_events_plan_time;
DROP INDEX IF EXISTS idx_acp_plan_mode_events_bear_time;

CREATE UNIQUE INDEX IF NOT EXISTS idx_client_plan_mode_one_open_session
    ON client_plan_mode_sessions (user_id, bear_id, client_session_id)
    WHERE state IN ('active', 'submitted');

CREATE INDEX IF NOT EXISTS idx_client_plan_mode_bear_session
    ON client_plan_mode_sessions (bear_slug, client_session_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_client_plan_mode_events_plan_time
    ON client_plan_mode_events (plan_mode_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_client_plan_mode_events_bear_time
    ON client_plan_mode_events (bear_id, created_at DESC);

COMMENT ON TABLE client_plan_mode_sessions IS 'Client session plan-mode gates for read-only planning and user approval before mutation.';
COMMENT ON COLUMN client_plan_mode_sessions.plan_artifact_path IS 'Durable markdown plan artifact path, e.g. pair/plans/client-<session>-<id>.md.';
COMMENT ON TABLE client_plan_mode_events IS 'Append-only audit stream for client plan-mode state changes.';
