use crate::integration_tests::{seed_user_and_bear, test_pool, two_task_job};
use crate::{
    DocketExecutionAttemptAuthorize, DocketExecutionAttemptOwner, DocketExecutionAttemptStart,
    DocketExecutionAttemptState, DocketPairBoundedOutcome, DocketPairBoundedOutcomeReport,
    DocketPairContinuationDecision, DocketService, PgDocketService,
};
use uuid::Uuid;

#[tokio::test]
async fn execution_attempt_authorization_and_start_are_idempotent_and_fenced() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping postgres-backed docket integration test; database unavailable");
        return;
    };
    let (user_id, bear_id) = seed_user_and_bear(&pool, "execution-attempt").await;
    let service = PgDocketService::from_pool(&pool);
    let job = service
        .create_job(two_task_job(user_id, bear_id))
        .await
        .expect("create job");
    let task_id = job.tasks[0].id;
    let authorization_key = Uuid::new_v4();
    let request = DocketExecutionAttemptAuthorize {
        bear_id,
        task_id,
        owner: DocketExecutionAttemptOwner::Pair {
            session_id: format!("pair-{}", Uuid::new_v4()),
            pair_run_id: Uuid::new_v4().to_string(),
        },
        authorization_key,
    };

    let authorized = service
        .authorize_execution_attempt(request.clone())
        .await
        .expect("authorize attempt");
    let replay = service
        .authorize_execution_attempt(request)
        .await
        .expect("replay authorization");
    assert_eq!(authorized.id, replay.id);
    assert_eq!(authorized.state, DocketExecutionAttemptState::Authorized);
    assert_eq!(authorized.fence_epoch, 1);

    let started = service
        .start_execution_attempt(DocketExecutionAttemptStart {
            attempt_id: authorized.id,
            fence_epoch: authorized.fence_epoch,
        })
        .await
        .expect("start attempt");
    assert_eq!(started.state, DocketExecutionAttemptState::Running);
    assert!(started.started_at.is_some());
    let restarted = service
        .start_execution_attempt(DocketExecutionAttemptStart {
            attempt_id: authorized.id,
            fence_epoch: authorized.fence_epoch,
        })
        .await
        .expect("replay start");
    assert_eq!(restarted.started_at, started.started_at);
    assert!(service
        .start_execution_attempt(DocketExecutionAttemptStart {
            attempt_id: authorized.id,
            fence_epoch: authorized.fence_epoch + 1,
        })
        .await
        .is_err());

    let conflicting = service
        .authorize_execution_attempt(DocketExecutionAttemptAuthorize {
            bear_id,
            task_id,
            owner: DocketExecutionAttemptOwner::Pair {
                session_id: format!("other-pair-{}", Uuid::new_v4()),
                pair_run_id: Uuid::new_v4().to_string(),
            },
            authorization_key: Uuid::new_v4(),
        })
        .await;
    assert!(conflicting.is_err(), "only one live attempt may own a task");
}

#[tokio::test]
async fn pair_bounded_outcomes_are_fenced_and_choose_canonical_yields() {
    let Some(pool) = test_pool().await else {
        eprintln!("skipping postgres-backed docket integration test; database unavailable");
        return;
    };
    let (user_id, bear_id) = seed_user_and_bear(&pool, "pair-bounded-outcome").await;
    let service = PgDocketService::from_pool(&pool);
    let job = service
        .create_job(two_task_job(user_id, bear_id))
        .await
        .expect("create job");
    let authorized = service
        .authorize_execution_attempt(DocketExecutionAttemptAuthorize {
            bear_id,
            task_id: job.tasks[0].id,
            owner: DocketExecutionAttemptOwner::Pair {
                session_id: format!("pair-{}", Uuid::new_v4()),
                pair_run_id: Uuid::new_v4().to_string(),
            },
            authorization_key: Uuid::new_v4(),
        })
        .await
        .expect("authorize attempt");
    let running = service
        .start_execution_attempt(DocketExecutionAttemptStart {
            attempt_id: authorized.id,
            fence_epoch: authorized.fence_epoch,
        })
        .await
        .expect("start attempt");
    let progress = DocketPairBoundedOutcomeReport {
        attempt_id: running.id,
        fence_epoch: running.fence_epoch,
        outcome: DocketPairBoundedOutcome::Progress,
    };
    let continued = service
        .report_pair_bounded_outcome(progress.clone())
        .await
        .expect("progress");
    assert_eq!(continued.decision, DocketPairContinuationDecision::Continue);
    assert_eq!(
        continued.attempt.state,
        DocketExecutionAttemptState::Running
    );
    assert_eq!(
        service
            .report_pair_bounded_outcome(progress)
            .await
            .expect("replay")
            .decision,
        DocketPairContinuationDecision::Continue
    );
    assert!(service
        .report_pair_bounded_outcome(DocketPairBoundedOutcomeReport {
            attempt_id: running.id,
            fence_epoch: running.fence_epoch + 1,
            outcome: DocketPairBoundedOutcome::AwaitingUser,
        })
        .await
        .is_err());
    let awaiting = service
        .report_pair_bounded_outcome(DocketPairBoundedOutcomeReport {
            attempt_id: running.id,
            fence_epoch: running.fence_epoch,
            outcome: DocketPairBoundedOutcome::AwaitingUser,
        })
        .await
        .expect("await user");
    assert_eq!(awaiting.decision, DocketPairContinuationDecision::AwaitUser);
    assert_eq!(
        awaiting.attempt.state,
        DocketExecutionAttemptState::AwaitingUser
    );
}
