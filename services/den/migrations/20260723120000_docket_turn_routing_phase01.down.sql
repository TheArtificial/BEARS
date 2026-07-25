DROP TABLE IF EXISTS docket_result_rollups;
DROP TABLE IF EXISTS docket_attention;
DROP INDEX IF EXISTS bear_task_run_state_one_in_progress_per_run;
DROP TABLE IF EXISTS docket_turn_attempts;
DROP TABLE IF EXISTS docket_routing_decisions;
DROP TABLE IF EXISTS docket_cursors;
DROP TABLE IF EXISTS docket_conversation_binding_runs;
DROP TABLE IF EXISTS docket_conversation_bindings;

ALTER TABLE bear_work_runs DROP CONSTRAINT IF EXISTS bear_work_runs_state_check;
ALTER TABLE bear_work_runs ADD CONSTRAINT bear_work_runs_state_check CHECK (
    state IN ('queued', 'claimed', 'provisioning', 'running', 'reporting',
              'succeeded', 'blocked', 'failed', 'cancelled', 'timed_out')
);

ALTER TABLE bear_tasks
    DROP COLUMN IF EXISTS result_rollup_policy,
    DROP COLUMN IF EXISTS expected_context_size,
    DROP COLUMN IF EXISTS routing_strategy;
