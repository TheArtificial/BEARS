-- ADR-0043: source/session references are client-session concepts, not ACP.

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'pair_reflection_runs'
          AND column_name = 'acp_session_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'pair_reflection_runs'
          AND column_name = 'client_session_id'
    ) THEN
        ALTER TABLE pair_reflection_runs RENAME COLUMN acp_session_id TO client_session_id;
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'conversations'
          AND column_name = 'source_acp_session_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'conversations'
          AND column_name = 'source_client_session_id'
    ) THEN
        ALTER TABLE conversations RENAME COLUMN source_acp_session_id TO source_client_session_id;
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'bear_work_plans'
          AND column_name = 'source_acp_session_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'bear_work_plans'
          AND column_name = 'source_client_session_id'
    ) THEN
        ALTER TABLE bear_work_plans RENAME COLUMN source_acp_session_id TO source_client_session_id;
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'docket_execution_sessions'
          AND column_name = 'source_acp_session_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'docket_execution_sessions'
          AND column_name = 'source_client_session_id'
    ) THEN
        ALTER TABLE docket_execution_sessions RENAME COLUMN source_acp_session_id TO source_client_session_id;
    END IF;
END $$;

DROP INDEX IF EXISTS idx_pair_reflection_runs_bear_session_time;
DROP INDEX IF EXISTS idx_bear_work_plans_bear_source_acp_session;
DROP INDEX IF EXISTS idx_docket_execution_sessions_source_acp_session;

CREATE INDEX IF NOT EXISTS idx_pair_reflection_runs_bear_client_session_time
    ON pair_reflection_runs (bear_id, user_id, client_session_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_bear_work_plans_bear_source_client_session
    ON bear_work_plans (bear_id, source_client_session_id)
    WHERE source_client_session_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_docket_execution_sessions_source_client_session
    ON docket_execution_sessions (bear_id, source_client_session_id, state, updated_at DESC)
    WHERE source_client_session_id IS NOT NULL;
