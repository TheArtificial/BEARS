-- Revert runtime role rename `chat` -> `talk`.

ALTER TABLE bear_agents DROP CONSTRAINT IF EXISTS bear_agents_role_check;

ALTER TABLE bear_skills_manifest DROP CONSTRAINT IF EXISTS bear_skills_manifest_applies_to_roles_check;
ALTER TABLE bear_skills_manifest DROP CONSTRAINT IF EXISTS bear_skills_manifest_check1;

ALTER TABLE bear_work_plans DROP CONSTRAINT IF EXISTS bear_work_plans_owner_role_check;

ALTER TABLE bear_work_plan_events DROP CONSTRAINT IF EXISTS bear_work_plan_events_actor_role_check;

ALTER TABLE bear_memory_proposals DROP CONSTRAINT IF EXISTS bear_memory_proposals_source_role_check;
ALTER TABLE bear_memory_proposals DROP CONSTRAINT IF EXISTS bear_memory_proposals_reviewer_role_check;

UPDATE bear_agents SET role = 'talk' WHERE role = 'chat';

UPDATE bear_work_plans SET owner_role = 'talk' WHERE owner_role = 'chat';

UPDATE bear_work_plan_events SET actor_role = 'talk' WHERE actor_role = 'chat';

UPDATE bear_memory_proposals SET source_role = 'talk' WHERE source_role = 'chat';

UPDATE bear_memory_proposals SET reviewer_role = 'talk' WHERE reviewer_role = 'chat';

UPDATE bear_skills_manifest
SET applies_to_roles = (
    SELECT COALESCE(array_agg(CASE WHEN r = 'chat' THEN 'talk' ELSE r END), ARRAY[]::TEXT[])
    FROM unnest(applies_to_roles) AS r
)
WHERE 'chat' = ANY (applies_to_roles);

ALTER TABLE bear_agents
    ADD CONSTRAINT bear_agents_role_check
    CHECK (role IN ('talk', 'pair', 'curate', 'work', 'watch'));

ALTER TABLE bear_skills_manifest
    ADD CONSTRAINT bear_skills_manifest_applies_to_roles_check
    CHECK (applies_to_roles <@ ARRAY['talk', 'pair', 'curate', 'work', 'watch']::TEXT[]);

ALTER TABLE bear_work_plans
    ADD CONSTRAINT bear_work_plans_owner_role_check
    CHECK (owner_role IN ('talk', 'pair', 'curate', 'work', 'watch'));

ALTER TABLE bear_work_plan_events
    ADD CONSTRAINT bear_work_plan_events_actor_role_check
    CHECK (actor_role IS NULL OR actor_role IN ('talk', 'pair', 'curate', 'work', 'watch'));

ALTER TABLE bear_memory_proposals
    ADD CONSTRAINT bear_memory_proposals_source_role_check
    CHECK (source_role IN ('talk', 'pair', 'curate', 'work', 'watch'));

ALTER TABLE bear_memory_proposals
    ADD CONSTRAINT bear_memory_proposals_reviewer_role_check
    CHECK (reviewer_role IS NULL OR reviewer_role IN ('talk', 'pair', 'curate', 'work', 'watch'));
