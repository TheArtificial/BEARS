ALTER TABLE bears
    ADD COLUMN IF NOT EXISTS work_enabled BOOLEAN NOT NULL DEFAULT true;

COMMENT ON COLUMN bears.work_enabled IS 'Enables durable work features for this Bear: task/job tools, work stance, and freeform task definition.';
