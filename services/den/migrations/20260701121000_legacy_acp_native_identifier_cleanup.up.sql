-- Cleanup after ACP/native-to-armature/client/runtime renames.
-- PostgreSQL keeps many constraint and backing index names when tables/columns are renamed,
-- so the final schema could have protocol-neutral tables/columns with stale internal names.

CREATE OR REPLACE FUNCTION pg_temp.rename_public_constraint(
    p_table_name TEXT,
    p_old_name TEXT,
    p_new_name TEXT
) RETURNS VOID
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid = con.conrelid
        JOIN pg_namespace ns ON ns.oid = cls.relnamespace
        WHERE ns.nspname = 'public'
          AND cls.relname = p_table_name
          AND con.conname = p_old_name
    ) AND NOT EXISTS (
        SELECT 1
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid = con.conrelid
        JOIN pg_namespace ns ON ns.oid = cls.relnamespace
        WHERE ns.nspname = 'public'
          AND cls.relname = p_table_name
          AND con.conname = p_new_name
    ) THEN
        EXECUTE format(
            'ALTER TABLE public.%I RENAME CONSTRAINT %I TO %I',
            p_table_name,
            p_old_name,
            p_new_name
        );
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION pg_temp.rename_public_index(
    p_old_name TEXT,
    p_new_name TEXT
) RETURNS VOID
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_class cls
        JOIN pg_namespace ns ON ns.oid = cls.relnamespace
        WHERE ns.nspname = 'public'
          AND cls.relkind = 'i'
          AND cls.relname = p_old_name
    ) AND NOT EXISTS (
        SELECT 1
        FROM pg_class cls
        JOIN pg_namespace ns ON ns.oid = cls.relnamespace
        WHERE ns.nspname = 'public'
          AND cls.relkind = 'i'
          AND cls.relname = p_new_name
    ) THEN
        EXECUTE format('ALTER INDEX public.%I RENAME TO %I', p_old_name, p_new_name);
    END IF;
END;
$$;

-- Armature token tables.
SELECT pg_temp.rename_public_constraint('armature_tokens', 'acp_tokens_pkey', 'armature_tokens_pkey');
SELECT pg_temp.rename_public_constraint('armature_tokens', 'acp_tokens_token_hash_key', 'armature_tokens_token_hash_key');
SELECT pg_temp.rename_public_constraint('armature_tokens', 'acp_tokens_user_id_fkey', 'armature_tokens_user_id_fkey');
SELECT pg_temp.rename_public_constraint('armature_token_bears', 'acp_token_bears_pkey', 'armature_token_bears_pkey');
SELECT pg_temp.rename_public_constraint('armature_token_bears', 'acp_token_bears_bear_id_fkey', 'armature_token_bears_bear_id_fkey');
SELECT pg_temp.rename_public_constraint('armature_token_bears', 'acp_token_bears_token_id_fkey', 'armature_token_bears_token_id_fkey');
SELECT pg_temp.rename_public_index('idx_acp_token_bears_bear_id', 'idx_armature_token_bears_bear_id');

-- Client session tables.
SELECT pg_temp.rename_public_constraint('client_sessions', 'acp_sessions_pkey', 'client_sessions_pkey');
SELECT pg_temp.rename_public_constraint('client_sessions', 'acp_sessions_bear_id_fkey', 'client_sessions_bear_id_fkey');
SELECT pg_temp.rename_public_constraint('client_sessions', 'acp_sessions_user_id_fkey', 'client_sessions_user_id_fkey');
SELECT pg_temp.rename_public_constraint('client_sessions', 'acp_sessions_current_mode_check', 'client_sessions_current_mode_check');

-- Client plan-mode tables.
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_pkey', 'client_plan_mode_sessions_pkey');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_acp_session_id_check', 'client_plan_mode_sessions_client_session_id_check');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_approved_by_user_id_fkey', 'client_plan_mode_sessions_approved_by_user_id_fkey');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_bear_id_fkey', 'client_plan_mode_sessions_bear_id_fkey');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_bear_slug_check', 'client_plan_mode_sessions_bear_slug_check');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_plan_artifact_path_check', 'client_plan_mode_sessions_plan_artifact_path_check');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_plan_body_check', 'client_plan_mode_sessions_plan_body_check');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_plan_title_check', 'client_plan_mode_sessions_plan_title_check');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_requested_by_check', 'client_plan_mode_sessions_requested_by_check');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_state_check', 'client_plan_mode_sessions_state_check');
SELECT pg_temp.rename_public_constraint('client_plan_mode_sessions', 'acp_plan_mode_sessions_user_id_fkey', 'client_plan_mode_sessions_user_id_fkey');

SELECT pg_temp.rename_public_constraint('client_plan_mode_events', 'acp_plan_mode_events_pkey', 'client_plan_mode_events_pkey');
SELECT pg_temp.rename_public_constraint('client_plan_mode_events', 'acp_plan_mode_events_bear_id_fkey', 'client_plan_mode_events_bear_id_fkey');
SELECT pg_temp.rename_public_constraint('client_plan_mode_events', 'acp_plan_mode_events_event_payload_check', 'client_plan_mode_events_event_payload_check');
SELECT pg_temp.rename_public_constraint('client_plan_mode_events', 'acp_plan_mode_events_event_type_check', 'client_plan_mode_events_event_type_check');
SELECT pg_temp.rename_public_constraint('client_plan_mode_events', 'acp_plan_mode_events_plan_mode_id_fkey', 'client_plan_mode_events_plan_mode_id_fkey');
SELECT pg_temp.rename_public_constraint('client_plan_mode_events', 'acp_plan_mode_events_user_id_fkey', 'client_plan_mode_events_user_id_fkey');

-- Runtime approval table.
SELECT pg_temp.rename_public_constraint('runtime_approvals', 'native_runtime_approvals_pkey', 'runtime_approvals_pkey');
SELECT pg_temp.rename_public_constraint('runtime_approvals', 'native_runtime_approvals_status_check', 'runtime_approvals_status_check');

-- Duplicate/obsolete source-session indexes left by older names. Replacement indexes were added in
-- 20260628150000_client_session_source_columns.up.sql.
DROP INDEX IF EXISTS idx_bear_work_plans_source_acp_session;
DROP INDEX IF EXISTS idx_docket_execution_sessions_acp;
