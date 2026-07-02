use den_runtime::{bearwire_obligations, bearwire_runs};
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

    bearwire_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");

    let tool = bearwire_obligations::upsert_tool_call_obligation(
        &pool,
        &run_id,
        &session_id,
        tool_call_id,
        None,
        serde_json::json!({ "phase": "tool" }),
    )
    .await
    .expect("create tool obligation");

    let permission = bearwire_obligations::upsert_permission_obligation(
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
        bearwire_obligations::get_permission_obligation(&pool, &run_id, permission_id)
            .await
            .expect("load by permission id")
            .expect("permission obligation exists");
    assert_eq!(by_permission.id, permission.id);

    let waiting_for_tool = bearwire_obligations::mark_waiting_for_tool_result(&pool, permission.id)
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
