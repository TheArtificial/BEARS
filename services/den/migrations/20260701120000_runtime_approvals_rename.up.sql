DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.tables
        WHERE table_schema = 'public'
          AND table_name = 'native_runtime_approvals'
    ) AND NOT EXISTS (
        SELECT 1
        FROM information_schema.tables
        WHERE table_schema = 'public'
          AND table_name = 'runtime_approvals'
    ) THEN
        ALTER TABLE native_runtime_approvals RENAME TO runtime_approvals;
    END IF;
END $$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'runtime_approvals'
          AND column_name = 'acp_session_id'
    ) AND NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'runtime_approvals'
          AND column_name = 'client_session_id'
    ) THEN
        ALTER TABLE runtime_approvals RENAME COLUMN acp_session_id TO client_session_id;
    END IF;
END $$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_indexes
        WHERE schemaname = 'public'
          AND indexname = 'idx_native_runtime_approvals_pending'
    ) AND NOT EXISTS (
        SELECT 1
        FROM pg_indexes
        WHERE schemaname = 'public'
          AND indexname = 'idx_runtime_approvals_pending'
    ) THEN
        ALTER INDEX idx_native_runtime_approvals_pending RENAME TO idx_runtime_approvals_pending;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_runtime_approvals_client_session
    ON runtime_approvals (bear_id, client_session_id, status, created_at);

COMMENT ON TABLE runtime_approvals IS 'Runtime approval requests awaiting armature/client decisions.';
COMMENT ON COLUMN runtime_approvals.client_session_id IS 'Protocol-neutral client session identifier that owns the approval request.';
