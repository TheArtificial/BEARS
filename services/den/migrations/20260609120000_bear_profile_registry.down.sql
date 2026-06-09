-- Revert Phase 6 profile registry migration.

ALTER TABLE bear_skills_manifest DROP CONSTRAINT IF EXISTS bear_skills_manifest_applies_to_profiles_check;

ALTER TABLE bear_skills_manifest RENAME COLUMN applies_to_profiles TO applies_to_roles;

ALTER TABLE bear_skills_manifest
    ADD CONSTRAINT bear_skills_manifest_applies_to_roles_check
    CHECK (applies_to_roles <@ ARRAY['chat', 'pair', 'curate', 'work', 'watch']::TEXT[]);

ALTER INDEX IF EXISTS idx_prompt_memory_blocks_bear_profile_scope
    RENAME TO idx_prompt_memory_blocks_bear_role_scope;

ALTER TABLE prompt_memory_blocks RENAME COLUMN profile_slug TO role_slug;

ALTER INDEX IF EXISTS idx_bear_profile_bindings_provisioning_status
    RENAME TO idx_bear_agents_provisioning_status;

ALTER INDEX IF EXISTS idx_bear_profile_bindings_profile
    RENAME TO idx_bear_agents_role;

ALTER INDEX IF EXISTS idx_bear_profile_bindings_letta_agent_id_unique
    RENAME TO idx_bear_agents_letta_agent_id_unique;

ALTER TABLE bear_profile_bindings DROP CONSTRAINT IF EXISTS bear_profile_bindings_profile_check;

ALTER TABLE bear_profile_bindings DROP COLUMN binding_id;

ALTER TABLE bear_profile_bindings RENAME COLUMN profile TO role;

ALTER TABLE bear_profile_bindings RENAME TO bear_agents;

ALTER TABLE bear_agents
    ADD CONSTRAINT bear_agents_role_check
    CHECK (role IN ('chat', 'pair', 'curate', 'work', 'watch'));
