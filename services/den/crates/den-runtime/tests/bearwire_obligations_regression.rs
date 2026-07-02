use den_runtime::{turn_obligations, turn_steps, turn_runs};
use uuid::Uuid;

async fn create_user_and_bear(pool: &sqlx::PgPool) -> (i32, Uuid) {
    let suffix = Uuid::new_v4().simple().to_string();
    let username = format!("obl{}", &suffix[..16]);
    let email = format!("{username}@example.test");
    let (user_id,): (i32,) = sqlx::query_as(
        r#"
        INSERT INTO users (email, username, display_name, passhash)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(email)
    .bind(&username)
    .bind("Obligation Test")
    .bind("test-passhash")
    .fetch_one(pool)
    .await
    .expect("create test user");

    let bear_id = Uuid::new_v4();
    let slug = format!("bear-{}", &suffix[..16]);
    sqlx::query(
        r#"
        INSERT INTO bears (id, slug, name)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(bear_id)
    .bind(slug)
    .bind("Obligation Test Bear")
    .execute(pool)
    .await
    .expect("create test bear");

    (user_id, bear_id)
}

#[sqlx::test(migrations = "../../migrations")]
async fn permission_obligation_promotes_existing_tool_obligation(pool: sqlx::PgPool) {
    let (user_id, bear_id) = create_user_and_bear(&pool).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let tool_call_id = "call-promote";
    let permission_id = "perm-promote";

    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");

    let tool = turn_obligations::upsert_tool_call_obligation(
        &pool,
        &run_id,
        &session_id,
        tool_call_id,
        None,
        serde_json::json!({ "phase": "tool" }),
    )
    .await
    .expect("create tool obligation");

    let permission = turn_obligations::upsert_permission_obligation(
        &pool,
        &run_id,
        &session_id,
        permission_id,
        Some(tool_call_id),
        serde_json::json!({ "phase": "permission" }),
    )
    .await
    .expect("promote to permission obligation");

    assert_eq!(permission.id, tool.id);
    assert_eq!(permission.kind, "permission");
    assert_eq!(
        permission.expected_client_method,
        "client.permission.result"
    );
    assert_eq!(permission.tool_call_id.as_deref(), Some(tool_call_id));
    assert_eq!(permission.permission_id.as_deref(), Some(permission_id));

    let by_permission =
        turn_obligations::get_permission_obligation(&pool, &run_id, permission_id)
            .await
            .expect("load by permission id")
            .expect("permission obligation exists");
    assert_eq!(by_permission.id, permission.id);

    let waiting_for_tool = turn_obligations::mark_waiting_for_tool_result(&pool, permission.id)
        .await
        .expect("mark waiting for tool result")
        .expect("obligation still open");
    assert_eq!(waiting_for_tool.id, permission.id);
    assert_eq!(waiting_for_tool.kind, "tool_call");
    assert_eq!(
        waiting_for_tool.expected_client_method,
        "client.tool.result"
    );
    assert_eq!(waiting_for_tool.tool_call_id.as_deref(), Some(tool_call_id));
    assert_eq!(
        waiting_for_tool.permission_id.as_deref(),
        Some(permission_id)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn open_obligation_barrier_counts_only_unsettled_client_waits(pool: sqlx::PgPool) {
    let (user_id, bear_id) = create_user_and_bear(&pool).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());

    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");

    let first = turn_obligations::upsert_tool_call_obligation(
        &pool,
        &run_id,
        &session_id,
        "call-first",
        None,
        serde_json::json!({ "tool": "first" }),
    )
    .await
    .expect("create first obligation");
    let second = turn_obligations::upsert_tool_call_obligation(
        &pool,
        &run_id,
        &session_id,
        "call-second",
        None,
        serde_json::json!({ "tool": "second" }),
    )
    .await
    .expect("create second obligation");

    turn_obligations::mark_result_received(
        &pool,
        first.id,
        serde_json::json!({ "status": "ok" }),
    )
    .await
    .expect("mark first received")
    .expect("first still open before receive");

    let open = turn_obligations::open_client_obligations_for_run(&pool, &run_id)
        .await
        .expect("list open obligations");
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, second.id);
    assert_eq!(open[0].tool_call_id.as_deref(), Some("call-second"));

    turn_obligations::mark_result_received(
        &pool,
        second.id,
        serde_json::json!({ "status": "ok" }),
    )
    .await
    .expect("mark second received")
    .expect("second still open before receive");

    let open = turn_obligations::open_client_obligations_for_run(&pool, &run_id)
        .await
        .expect("list open obligations after all received");
    assert!(open.is_empty(), "all tool results are received: {open:#?}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn step_barrier_counts_only_obligations_for_same_step(pool: sqlx::PgPool) {
    let (user_id, bear_id) = create_user_and_bear(&pool).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());

    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");

    let first_step = turn_steps::ensure_active_step(&pool, &run_id)
        .await
        .expect("ensure first step");
    let first = turn_obligations::upsert_tool_call_obligation_for_step(
        &pool,
        &run_id,
        &session_id,
        Some(first_step.id),
        "call-first-step",
        None,
        serde_json::json!({ "tool": "first-step" }),
    )
    .await
    .expect("create first step obligation");
    assert_eq!(first.turn_step_id, Some(first_step.id));

    turn_steps::transition_step(&pool, first_step.id, "continued")
        .await
        .expect("close first step");
    let second_step = turn_steps::ensure_active_step(&pool, &run_id)
        .await
        .expect("ensure second step");
    assert_ne!(first_step.id, second_step.id);
    let second = turn_obligations::upsert_tool_call_obligation_for_step(
        &pool,
        &run_id,
        &session_id,
        Some(second_step.id),
        "call-second-step",
        None,
        serde_json::json!({ "tool": "second-step" }),
    )
    .await
    .expect("create second step obligation");

    let open_first_step =
        turn_obligations::open_client_obligations_for_step(&pool, first_step.id)
            .await
            .expect("list first step open obligations");
    assert_eq!(open_first_step.len(), 1);
    assert_eq!(open_first_step[0].id, first.id);

    let open_second_step =
        turn_obligations::open_client_obligations_for_step(&pool, second_step.id)
            .await
            .expect("list second step open obligations");
    assert_eq!(open_second_step.len(), 1);
    assert_eq!(open_second_step[0].id, second.id);
}
