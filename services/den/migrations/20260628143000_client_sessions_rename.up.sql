-- ADR-0043: session persistence is a Den client-session concept, not ACP.
-- Rename the live table/column while preserving data and existing foreign keys.

DO $$
BEGIN
    IF to_regclass('public.client_sessions') IS NULL
       AND to_regclass('public.acp_sessions') IS NOT NULL THEN
        ALTER TABLE acp_sessions RENAME TO client_sessions;
    END IF;
END $$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'client_sessions'
          AND column_name = 'acp_session_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'client_sessions'
          AND column_name = 'client_session_id'
    ) THEN
        ALTER TABLE client_sessions RENAME COLUMN acp_session_id TO client_session_id;
    END IF;
END $$;

ALTER TABLE client_sessions
    DROP CONSTRAINT IF EXISTS acp_sessions_user_id_bear_id_acp_session_id_key;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'client_sessions_user_bear_client_session_key'
    ) THEN
        ALTER TABLE client_sessions
            ADD CONSTRAINT client_sessions_user_bear_client_session_key
            UNIQUE (user_id, bear_id, client_session_id);
    END IF;
END $$;

DROP INDEX IF EXISTS idx_acp_sessions_bear_session;
DROP INDEX IF EXISTS idx_acp_sessions_conversation_id;

CREATE INDEX IF NOT EXISTS idx_client_sessions_bear_session
    ON client_sessions (bear_slug, client_session_id);

CREATE INDEX IF NOT EXISTS idx_client_sessions_conversation_id
    ON client_sessions (conversation_id);

COMMENT ON TABLE client_sessions IS 'Client session bindings mapped to pair-role runtime conversations for lifecycle handling.';
COMMENT ON COLUMN client_sessions.client_session_id IS 'Protocol-neutral client session identifier.';
COMMENT ON COLUMN client_sessions.runtime_session_id IS 'Runtime-neutral client session binding id. Historical deployments called this codepool_session_id.';
COMMENT ON COLUMN client_sessions.current_mode IS 'Den-mediated client session mode: ask, plan, or write. Client requests are validated by Den before this changes.';
COMMENT ON COLUMN client_sessions.adapter_environment IS 'Latest adapter-published environment snapshot for this client session. This is a BearWire-like runtime report owned by the trusted adapter/edge process.';
