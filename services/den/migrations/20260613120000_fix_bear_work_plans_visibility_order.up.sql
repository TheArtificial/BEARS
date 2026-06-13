-- Forward fix for a constraint-ordering bug in
-- 20260610120000_profile_vocabulary_memory_and_plans.up.sql.
--
-- That migration added `bear_work_plans_visibility_check` (new vocabulary)
-- BEFORE the UPDATE that migrates the legacy value 'private_to_role' to
-- 'private_to_profile', so the ADD CONSTRAINT fails on any database that still
-- holds pre-vocabulary rows. We do not edit the original migration (sqlx records
-- its checksum once applied, and editing it would break already-migrated DBs);
-- instead we reassert the constraint here in the correct order: drop, migrate
-- data, then re-add.
--
-- Scope note: this only repairs databases that already applied 20260610120000
-- (fresh databases never hit the bug because the affected tables are empty). A
-- database still stuck on the failing 20260610120000 must have its data repaired
-- manually first, because sqlx halts on the failed migration and never reaches
-- this one.

ALTER TABLE bear_work_plans DROP CONSTRAINT IF EXISTS bear_work_plans_visibility_check;

UPDATE bear_work_plans
SET visibility = 'private_to_profile'
WHERE visibility = 'private_to_role';

ALTER TABLE bear_work_plans
    ADD CONSTRAINT bear_work_plans_visibility_check
    CHECK (visibility IN ('private_to_profile', 'same_user', 'bear_visible', 'handoff_requested'));
