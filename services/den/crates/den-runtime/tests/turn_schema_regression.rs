#[sqlx::test(migrations = "../../migrations")]
async fn turn_core_schema_uses_neutral_names(pool: sqlx::PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT table_name
        FROM information_schema.tables
        WHERE table_schema = 'public'
          AND table_name IN (
            'turn_runs',
            'turn_steps',
            'turn_obligations',
            'turn_obligation_results',
            'bearwire_runs',
            'bearwire_run_obligations',
            'bearwire_client_results',
            'bearwire_events'
          )
        ORDER BY table_name
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("query turn/BearWire table names");

    assert!(
        tables.contains(&"turn_runs".to_string()),
        "turn_runs missing: {tables:#?}"
    );
    assert!(
        tables.contains(&"turn_steps".to_string()),
        "turn_steps missing: {tables:#?}"
    );
    assert!(
        tables.contains(&"turn_obligations".to_string()),
        "turn_obligations missing: {tables:#?}"
    );
    assert!(
        tables.contains(&"turn_obligation_results".to_string()),
        "turn_obligation_results missing: {tables:#?}"
    );
    assert!(
        tables.contains(&"bearwire_events".to_string()),
        "bearwire_events should remain the BearWire wire event log: {tables:#?}"
    );

    assert!(
        !tables.contains(&"bearwire_runs".to_string()),
        "core turn run table should have been renamed: {tables:#?}"
    );
    assert!(
        !tables.contains(&"bearwire_run_obligations".to_string()),
        "core turn obligation table should have been renamed: {tables:#?}"
    );
    assert!(
        !tables.contains(&"bearwire_client_results".to_string()),
        "core turn obligation result table should have been renamed: {tables:#?}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn turn_obligation_schema_supports_neutral_actions(pool: sqlx::PgPool) {
    let columns: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT column_name
        FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'turn_obligations'
          AND column_name IN (
            'expected_responder_action',
            'expected_client_method',
            'responder_ref_id',
            'turn_step_id',
            'step_id'
          )
        ORDER BY column_name
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("query turn_obligations columns");

    assert!(
        columns.contains(&"expected_responder_action".to_string()),
        "expected_responder_action missing: {columns:#?}"
    );
    assert!(
        columns.contains(&"responder_ref_id".to_string()),
        "responder_ref_id missing: {columns:#?}"
    );
    assert!(
        columns.contains(&"turn_step_id".to_string()),
        "turn_step_id missing: {columns:#?}"
    );
    assert!(
        !columns.contains(&"expected_client_method".to_string()),
        "core schema should not expose BearWire method column: {columns:#?}"
    );
    assert!(
        !columns.contains(&"step_id".to_string()),
        "core schema should use turn_step_id, not step_id: {columns:#?}"
    );

    let check_clause: Option<String> = sqlx::query_scalar(
        r#"
        SELECT check_clause
        FROM information_schema.check_constraints
        WHERE constraint_schema = 'public'
          AND constraint_name = 'turn_obligations_expected_responder_action_check'
        LIMIT 1
        "#,
    )
    .fetch_optional(&pool)
    .await
    .expect("query expected responder action check");
    let check_clause = check_clause.expect("expected responder action check exists");
    for value in [
        "tool_result",
        "permission_decision",
        "human_input",
        "resource_binding",
        "handoff_decision",
    ] {
        assert!(
            check_clause.contains(value),
            "check constraint missing {value}: {check_clause}"
        );
    }
}
