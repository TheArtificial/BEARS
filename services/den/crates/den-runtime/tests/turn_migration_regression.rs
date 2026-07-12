use sqlx::Row;

const TURN_RENAME_MIGRATION: &str =
    include_str!("../../../migrations/20260702110000_turn_obligations_rename.up.sql");

#[sqlx::test]
async fn turn_obligation_rename_migration_rewrites_legacy_values_before_new_checks(
    pool: sqlx::PgPool,
) {
    sqlx::raw_sql(
        r"
        CREATE TABLE bearwire_runs (
            id UUID PRIMARY KEY,
            run_id TEXT NOT NULL UNIQUE,
            session_id TEXT NOT NULL,
            bear_id UUID NOT NULL,
            user_id INTEGER NOT NULL,
            state TEXT NOT NULL DEFAULT 'accepted' CHECK (state IN (
                'accepted',
                'running',
                'waiting_for_tool_result',
                'waiting_for_permission',
                'continuing',
                'completed',
                'failed',
                'cancelled'
            )),
            terminal_reason TEXT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            completed_at TIMESTAMPTZ NULL
        );

        CREATE TABLE bearwire_run_obligations (
            id UUID PRIMARY KEY,
            run_id TEXT NOT NULL REFERENCES bearwire_runs(run_id) ON DELETE CASCADE,
            session_id TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('tool_call', 'permission')),
            expected_client_method TEXT NOT NULL CHECK (
                expected_client_method IN ('client.tool.result', 'client.permission.result')
            ),
            tool_call_id TEXT NULL,
            permission_id TEXT NULL,
            state TEXT NOT NULL CHECK (state IN (
                'requested',
                'waiting_for_client',
                'result_received',
                'continued',
                'failed',
                'cancelled'
            )),
            request_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
            result_payload JSONB NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            completed_at TIMESTAMPTZ NULL,
            CHECK (tool_call_id IS NOT NULL OR permission_id IS NOT NULL)
        );

        CREATE TABLE bearwire_client_results (
            id UUID PRIMARY KEY,
            run_id TEXT NOT NULL REFERENCES bearwire_runs(run_id) ON DELETE CASCADE,
            obligation_kind TEXT NOT NULL,
            obligation_id TEXT NOT NULL,
            result_hash TEXT NOT NULL,
            payload_json JSONB NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE (run_id, obligation_kind, obligation_id)
        );

        INSERT INTO bearwire_runs (id, run_id, session_id, bear_id, user_id, state)
        VALUES (
            '00000000-0000-0000-0000-000000000001',
            'run-legacy',
            'session-legacy',
            '00000000-0000-0000-0000-000000000002',
            1,
            'waiting_for_permission'
        );

        INSERT INTO bearwire_run_obligations (
            id, run_id, session_id, kind, expected_client_method,
            tool_call_id, permission_id, state
        ) VALUES
        (
            '00000000-0000-0000-0000-000000000003',
            'run-legacy',
            'session-legacy',
            'tool_call',
            'client.tool.result',
            'call-legacy',
            NULL,
            'waiting_for_client'
        ),
        (
            '00000000-0000-0000-0000-000000000004',
            'run-legacy',
            'session-legacy',
            'permission',
            'client.permission.result',
            'call-needs-approval',
            'perm-legacy',
            'waiting_for_client'
        );
        ",
    )
    .execute(&pool)
    .await
    .expect("create legacy turn schema");

    sqlx::raw_sql(TURN_RENAME_MIGRATION)
        .execute(&pool)
        .await
        .expect("apply turn rename migration over legacy rows");

    let rows = sqlx::query(
        r"
        SELECT kind, expected_responder_action
        FROM turn_obligations
        ORDER BY id
        ",
    )
    .fetch_all(&pool)
    .await
    .expect("load migrated obligations");

    let values: Vec<(String, String)> = rows
        .into_iter()
        .map(|row| (row.get("kind"), row.get("expected_responder_action")))
        .collect();
    assert_eq!(
        values,
        vec![
            ("tool_result".to_string(), "tool_result".to_string()),
            (
                "permission_decision".to_string(),
                "permission_decision".to_string()
            ),
        ]
    );

    let stale_constraints: i64 = sqlx::query_scalar(
        r"
        SELECT COUNT(*)::bigint
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid = con.conrelid
        JOIN pg_namespace ns ON ns.oid = cls.relnamespace
        WHERE ns.nspname = 'public'
          AND cls.relname = 'turn_obligations'
          AND con.conname IN (
              'bearwire_run_obligations_kind_check',
              'bearwire_run_obligations_expected_client_method_check'
          )
        ",
    )
    .fetch_one(&pool)
    .await
    .expect("count stale constraints");
    assert_eq!(stale_constraints, 0);
}
