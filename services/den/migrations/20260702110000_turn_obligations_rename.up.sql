-- ADR-0048: turn obligations are core runtime state, not BearWire wire state.
-- Rename deployed BearWire-prefixed obligation/result tables forward to neutral names.

DO $$
BEGIN
    IF to_regclass('public.turn_runs') IS NULL
       AND to_regclass('public.bearwire_runs') IS NOT NULL THEN
        ALTER TABLE bearwire_runs RENAME TO turn_runs;
    END IF;

    IF to_regclass('public.turn_obligations') IS NULL
       AND to_regclass('public.bearwire_run_obligations') IS NOT NULL THEN
        ALTER TABLE bearwire_run_obligations RENAME TO turn_obligations;
    END IF;

    IF to_regclass('public.turn_obligation_results') IS NULL
       AND to_regclass('public.bearwire_client_results') IS NOT NULL THEN
        ALTER TABLE bearwire_client_results RENAME TO turn_obligation_results;
    END IF;
END $$;

-- Rename old indexes where PostgreSQL retained their original names across table rename.
DO $$
BEGIN
    IF to_regclass('public.idx_bearwire_runs_one_active_per_session') IS NOT NULL
       AND to_regclass('public.idx_turn_runs_one_active_per_session') IS NULL THEN
        ALTER INDEX idx_bearwire_runs_one_active_per_session RENAME TO idx_turn_runs_one_active_per_session;
    END IF;

    IF to_regclass('public.idx_bearwire_runs_bear_created') IS NOT NULL
       AND to_regclass('public.idx_turn_runs_bear_created') IS NULL THEN
        ALTER INDEX idx_bearwire_runs_bear_created RENAME TO idx_turn_runs_bear_created;
    END IF;

    IF to_regclass('public.idx_bearwire_obligations_tool_call') IS NOT NULL
       AND to_regclass('public.idx_turn_obligations_tool_call') IS NULL THEN
        ALTER INDEX idx_bearwire_obligations_tool_call RENAME TO idx_turn_obligations_tool_call;
    END IF;

    IF to_regclass('public.idx_bearwire_obligations_permission') IS NOT NULL
       AND to_regclass('public.idx_turn_obligations_permission') IS NULL THEN
        ALTER INDEX idx_bearwire_obligations_permission RENAME TO idx_turn_obligations_permission;
    END IF;

    IF to_regclass('public.idx_bearwire_obligations_run_state') IS NOT NULL
       AND to_regclass('public.idx_turn_obligations_run_state') IS NULL THEN
        ALTER INDEX idx_bearwire_obligations_run_state RENAME TO idx_turn_obligations_run_state;
    END IF;

    IF to_regclass('public.idx_bearwire_obligations_session_state') IS NOT NULL
       AND to_regclass('public.idx_turn_obligations_session_state') IS NULL THEN
        ALTER INDEX idx_bearwire_obligations_session_state RENAME TO idx_turn_obligations_session_state;
    END IF;

    IF to_regclass('public.idx_bearwire_client_results_created') IS NOT NULL
       AND to_regclass('public.idx_turn_obligation_results_created') IS NULL THEN
        ALTER INDEX idx_bearwire_client_results_created RENAME TO idx_turn_obligation_results_created;
    END IF;
END $$;

-- Rename primary-key/unique/check constraints where possible. Constraint names do not affect
-- runtime behavior, but keeping them neutral makes schema inspection less misleading.
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

SELECT pg_temp.rename_public_constraint('turn_runs', 'bearwire_runs_pkey', 'turn_runs_pkey');
SELECT pg_temp.rename_public_constraint('turn_runs', 'bearwire_runs_run_id_key', 'turn_runs_run_id_key');
SELECT pg_temp.rename_public_constraint('turn_runs', 'bearwire_runs_bear_id_fkey', 'turn_runs_bear_id_fkey');
SELECT pg_temp.rename_public_constraint('turn_runs', 'bearwire_runs_user_id_fkey', 'turn_runs_user_id_fkey');
SELECT pg_temp.rename_public_constraint('turn_runs', 'bearwire_runs_state_check', 'turn_runs_state_check');

SELECT pg_temp.rename_public_constraint('turn_obligations', 'bearwire_run_obligations_pkey', 'turn_obligations_pkey');
SELECT pg_temp.rename_public_constraint('turn_obligations', 'bearwire_run_obligations_run_id_fkey', 'turn_obligations_run_id_fkey');
SELECT pg_temp.rename_public_constraint('turn_obligations', 'bearwire_run_obligations_kind_check', 'turn_obligations_kind_check');
SELECT pg_temp.rename_public_constraint('turn_obligations', 'bearwire_run_obligations_expected_client_method_check', 'turn_obligations_expected_client_method_check');
SELECT pg_temp.rename_public_constraint('turn_obligations', 'bearwire_run_obligations_state_check', 'turn_obligations_state_check');
SELECT pg_temp.rename_public_constraint('turn_obligations', 'bearwire_run_obligations_check', 'turn_obligations_tool_or_permission_check');

SELECT pg_temp.rename_public_constraint('turn_obligation_results', 'bearwire_client_results_pkey', 'turn_obligation_results_pkey');
SELECT pg_temp.rename_public_constraint('turn_obligation_results', 'bearwire_client_results_run_id_fkey', 'turn_obligation_results_run_id_fkey');
SELECT pg_temp.rename_public_constraint('turn_obligation_results', 'bearwire_client_results_run_id_obligation_kind_obligation_id_key', 'turn_obligation_results_run_kind_obligation_key');

COMMENT ON TABLE turn_runs IS 'Core turn/run lifecycle state used by the runtime coordinator. BearWire projects this state to trusted armatures.';
COMMENT ON TABLE turn_obligations IS 'Core turn obligations waiting on tools, permissions, humans, channels, or other responders.';
COMMENT ON TABLE turn_obligation_results IS 'Results submitted for core turn obligations, with idempotency hashing.';
