use den_runtime::{
    bearwire_events,
    surface_projection::{project_obligation_for_surface, SurfaceActionKind, TurnSurfaceKind},
    turn_obligations::{self, ExpectedResponderAction, TurnObligationKind},
    turn_runs, turn_steps, turn_waits,
};
use sqlx::Row;
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

    let tool = turn_obligations::upsert_tool_result_obligation(
        &pool,
        &run_id,
        &session_id,
        tool_call_id,
        None,
        serde_json::json!({ "phase": "tool" }),
    )
    .await
    .expect("create tool obligation");

    let permission = turn_obligations::upsert_permission_decision_obligation(
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
    assert_eq!(permission.kind, "permission_decision");
    assert_eq!(permission.expected_responder_action, "permission_decision");
    assert_eq!(permission.tool_call_id.as_deref(), Some(tool_call_id));
    assert_eq!(permission.permission_id.as_deref(), Some(permission_id));

    let by_permission = turn_obligations::get_permission_obligation(&pool, &run_id, permission_id)
        .await
        .expect("load by permission id")
        .expect("permission obligation exists");
    assert_eq!(by_permission.id, permission.id);

    let waiting_for_tool = turn_obligations::mark_waiting_for_tool_result(&pool, permission.id)
        .await
        .expect("mark waiting for tool result")
        .expect("obligation still open");
    assert_eq!(waiting_for_tool.id, permission.id);
    assert_eq!(waiting_for_tool.kind, "tool_result");
    assert_eq!(waiting_for_tool.expected_responder_action, "tool_result");
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

    let first = turn_obligations::upsert_tool_result_obligation(
        &pool,
        &run_id,
        &session_id,
        "call-first",
        None,
        serde_json::json!({ "tool": "first" }),
    )
    .await
    .expect("create first obligation");
    let second = turn_obligations::upsert_tool_result_obligation(
        &pool,
        &run_id,
        &session_id,
        "call-second",
        None,
        serde_json::json!({ "tool": "second" }),
    )
    .await
    .expect("create second obligation");

    turn_obligations::mark_result_received(&pool, first.id, serde_json::json!({ "status": "ok" }))
        .await
        .expect("mark first received")
        .expect("first still open before receive");

    let open = turn_obligations::open_client_obligations_for_run(&pool, &run_id)
        .await
        .expect("list open obligations");
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, second.id);
    assert_eq!(open[0].tool_call_id.as_deref(), Some("call-second"));

    turn_obligations::mark_result_received(&pool, second.id, serde_json::json!({ "status": "ok" }))
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
    let first = turn_obligations::upsert_tool_result_obligation_for_step(
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
    let second = turn_obligations::upsert_tool_result_obligation_for_step(
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

    let open_first_step = turn_obligations::open_client_obligations_for_step(&pool, first_step.id)
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

#[sqlx::test(migrations = "../../migrations")]
async fn core_obligations_support_non_bearwire_channel_waits(pool: sqlx::PgPool) {
    let (user_id, bear_id) = create_user_and_bear(&pool).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());

    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");
    let step = turn_steps::ensure_active_step(&pool, &run_id)
        .await
        .expect("ensure step");

    let cases = [
        (
            TurnObligationKind::HumanInput,
            ExpectedResponderAction::HumanInput,
            "slack-thread-reply-1",
        ),
        (
            TurnObligationKind::ResourceBinding,
            ExpectedResponderAction::ResourceBinding,
            "calendar-oauth-binding-1",
        ),
        (
            TurnObligationKind::HandoffDecision,
            ExpectedResponderAction::HandoffDecision,
            "work-handoff-1",
        ),
    ];

    for (kind, action, responder_ref_id) in cases {
        let row = turn_obligations::create_turn_obligation_for_step(
            &pool,
            &run_id,
            &session_id,
            Some(step.id),
            kind,
            action,
            responder_ref_id,
            serde_json::json!({ "responder_ref_id": responder_ref_id }),
        )
        .await
        .expect("create non-BearWire obligation");
        assert_eq!(row.kind, kind.as_str());
        assert_eq!(row.expected_responder_action, action.as_str());
        assert_eq!(row.responder_ref_id.as_deref(), Some(responder_ref_id));
        assert_eq!(row.turn_step_id, Some(step.id));
    }

    let open = turn_obligations::open_client_obligations_for_step(&pool, step.id)
        .await
        .expect("list open obligations");
    assert_eq!(open.len(), 3);
}

#[sqlx::test(migrations = "../../migrations")]
async fn transactional_tool_wait_persists_step_obligation_and_event(pool: sqlx::PgPool) {
    let (user_id, bear_id) = create_user_and_bear(&pool).await;
    let session_id = format!("session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let permission_id = Some(format!("perm_{}", Uuid::new_v4().simple()));
    let title = Some("Read a file".to_string());
    let kind = Some("function".to_string());
    let arguments = serde_json::json!({ "path": "README.md" });
    let approval_reason = Some("read workspace file".to_string());
    let event_run_id = Some(run_id.clone());

    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");

    let persisted = turn_waits::persist_bearwire_tool_call_wait_transactionally(
        &pool,
        turn_waits::PersistToolCallWaitInput {
            session_id: &session_id,
            run_id: &run_id,
            bear_id,
            user_id,
            request_id: Uuid::new_v4(),
            tool_call_id: "call-transactional-wait",
            tool_name: "fs_read_text_file",
            title: &title,
            kind: &kind,
            arguments: &arguments,
            approval_request_id: &permission_id,
            approval_required: true,
            approval_reason: &approval_reason,
            event_run_id: &event_run_id,
        },
    )
    .await
    .expect("persist wait transactionally");

    assert!(persisted.effective_approval_required);
    assert_eq!(persisted.obligation.kind, "permission_decision");
    assert_eq!(
        persisted.obligation.expected_responder_action,
        "permission_decision"
    );
    assert_eq!(
        persisted.obligation.permission_id.as_deref(),
        permission_id.as_deref()
    );
    assert_eq!(
        persisted.obligation.tool_call_id.as_deref(),
        Some("call-transactional-wait")
    );
    assert_eq!(
        persisted.obligation.turn_step_id,
        Some(persisted.turn_step_id)
    );

    let run = turn_runs::get_run(&pool, &run_id)
        .await
        .expect("load run")
        .expect("run exists");
    assert_eq!(run.state, "waiting_for_permission");

    let step_state: String = sqlx::query(
        r#"
        SELECT state
        FROM turn_steps
        WHERE id = $1
        "#,
    )
    .bind(persisted.turn_step_id)
    .fetch_one(&pool)
    .await
    .expect("load turn step")
    .get("state");
    assert_eq!(step_state, "waiting_for_client");

    let events = bearwire_events::list_bearwire_events_after(&pool, &session_id, None, 10)
        .await
        .expect("list events");
    let waiting = events
        .iter()
        .find(|row| row.event_type == "client.waiting")
        .expect("client.waiting event exists");
    assert_eq!(waiting.sequence_no, persisted.event_sequence);
    assert_eq!(
        waiting.event.data["obligation_id"],
        persisted.obligation.id.to_string()
    );
    assert_eq!(
        waiting.event.data["expected_client_method"],
        "client.permission.result"
    );
    assert_eq!(
        waiting.event.data["turn_step_id"],
        persisted.turn_step_id.to_string()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn web_chat_human_input_uses_core_surface_obligation_path(pool: sqlx::PgPool) {
    let (user_id, bear_id) = create_user_and_bear(&pool).await;
    let session_id = format!("web-chat-session-{}", Uuid::new_v4().simple());
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let responder_ref_id = format!("web-reply-{}", Uuid::new_v4().simple());

    turn_runs::create_run(&pool, &run_id, &session_id, bear_id, user_id)
        .await
        .expect("create run");

    let persisted = turn_waits::persist_surface_obligation_transactionally(
        &pool,
        turn_waits::PersistSurfaceObligationInput {
            session_id: &session_id,
            run_id: &run_id,
            kind: TurnObligationKind::HumanInput,
            expected_responder_action: ExpectedResponderAction::HumanInput,
            responder_ref_id: &responder_ref_id,
            request_payload: serde_json::json!({
                "surface": "web_chat",
                "prompt": "Please choose an option"
            }),
        },
    )
    .await
    .expect("persist web-chat human-input obligation");

    assert_eq!(persisted.obligation.kind, "human_input");
    assert_eq!(
        persisted.obligation.expected_responder_action,
        "human_input"
    );
    assert_eq!(
        persisted.obligation.responder_ref_id.as_deref(),
        Some(responder_ref_id.as_str())
    );
    assert_eq!(
        persisted.obligation.turn_step_id,
        Some(persisted.turn_step_id)
    );

    let run = turn_runs::get_run(&pool, &run_id)
        .await
        .expect("load run")
        .expect("run exists");
    assert_eq!(run.state, "waiting_for_client");

    let projection =
        project_obligation_for_surface(&persisted.obligation, TurnSurfaceKind::WebChat);
    assert_eq!(projection.action_kind, SurfaceActionKind::ChatReply);
    assert_eq!(
        projection.obligation_id,
        persisted.obligation.id.to_string()
    );
    assert_eq!(
        projection.responder_ref_id.as_deref(),
        Some(responder_ref_id.as_str())
    );
    assert_eq!(
        projection.payload["turn_step_id"],
        persisted.turn_step_id.to_string()
    );
}
