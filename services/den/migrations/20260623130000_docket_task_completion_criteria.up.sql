-- Lightweight task-level completion criteria for Docket tasks.
-- Criteria are required for Docket tasks so models have a concrete stopping target
-- instead of treating open-ended task bodies as permission to keep reading forever.

ALTER TABLE bear_tasks
    ADD COLUMN IF NOT EXISTS completion_criteria JSONB NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE bear_tasks
    DROP CONSTRAINT IF EXISTS bear_tasks_completion_criteria_shape_check;

ALTER TABLE bear_tasks
    ADD CONSTRAINT bear_tasks_completion_criteria_shape_check
    CHECK (jsonb_typeof(completion_criteria) = 'array');

COMMENT ON COLUMN bear_tasks.completion_criteria IS 'Array of non-empty completion criteria strings for the task; model-facing task creation requires at least one criterion.';
