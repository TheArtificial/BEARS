-- No-op: this migration only reasserts the `bear_work_plans_visibility_check`
-- constraint that 20260610120000 already owns (in the correct order). Reverting
-- it must not drop that constraint while 20260610120000 remains applied, so there
-- is nothing to undo here. Rolling back the vocabulary itself is handled by
-- 20260610120000_profile_vocabulary_memory_and_plans.down.sql.
SELECT 1;
