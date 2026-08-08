-- A Pair session owns one optional current Docket task. This is distinct from
-- durable Docket execution and from a Work run's assigned task/subtree.
ALTER TABLE client_sessions
    ADD COLUMN IF NOT EXISTS current_task_id UUID;

COMMENT ON COLUMN client_sessions.current_task_id IS
    'Optional selected Docket task that gives this Pair client session its current objective.';
