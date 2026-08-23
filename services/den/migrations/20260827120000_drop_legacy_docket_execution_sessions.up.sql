-- Pre-release cutover: execution authority is represented exclusively by
-- docket_execution_attempts. The session-scoped compatibility table and its
-- dependent scheduler-observation table have no remaining runtime consumers.
DROP TABLE IF EXISTS docket_scheduler_observations;
DROP TABLE IF EXISTS docket_execution_sessions;
