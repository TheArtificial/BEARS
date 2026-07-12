-- Retire legacy work-plan storage after task-list/job tooling moved to canonical tables.
-- This is intentionally destructive: legacy plan/event data is no longer read.

DROP TABLE IF EXISTS bear_work_plan_events;
DROP TABLE IF EXISTS bear_work_plans;
