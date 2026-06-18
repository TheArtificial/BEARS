-- Phase 6: Den-native profile registry — binding_id as canonical runtime identity.

ALTER TABLE bear_agents DROP CONSTRAINT IF EXISTS bear_agents_role_check;

ALTER TABLE bear_agents RENAME TO bear_profile_bindings;

ALTER TABLE bear_profile_bindings RENAME COLUMN role TO profile;

ALTER TABLE bear_profile_bindings ADD COLUMN binding_id TEXT;

UPDATE bear_profile_bindings
SET binding_id = COALESCE(
    NULLIF(btrim(letta_agent_id), ''),
    'den-native:' || bear_id::text || ':' || profile
)
WHERE binding_id IS NULL OR btrim(binding_id) = '';

ALTER TABLE bear_profile_bindings ALTER COLUMN binding_id SET NOT NULL;

ALTER TABLE bear_profile_bindings
    ADD CONSTRAINT bear_profile_bindings_profile_check
    CHECK (profile IN ('chat', 'pair', 'curate', 'work', 'watch'));

ALTER INDEX IF EXISTS idx_bear_agents_letta_agent_id_unique
    RENAME TO idx_bear_profile_bindings_letta_agent_id_unique;

ALTER INDEX IF EXISTS idx_bear_agents_role
    RENAME TO idx_bear_profile_bindings_profile;

ALTER INDEX IF EXISTS idx_bear_agents_provisioning_status
    RENAME TO idx_bear_profile_bindings_provisioning_status;

COMMENT ON COLUMN bear_profile_bindings.letta_agent_id IS 'Deprecated: legacy Letta agent id only; canonical runtime identity is binding_id.';
COMMENT ON TABLE bear_profile_bindings IS 'Den-owned per-profile runtime registry (binding id, config hash, provisioning status).';

ALTER TABLE prompt_memory_blocks RENAME COLUMN role_slug TO profile_slug;

ALTER INDEX IF EXISTS idx_prompt_memory_blocks_bear_role_scope
    RENAME TO idx_prompt_memory_blocks_bear_profile_scope;

ALTER TABLE bear_skills_manifest DROP CONSTRAINT IF EXISTS bear_skills_manifest_applies_to_roles_check;

ALTER TABLE bear_skills_manifest RENAME COLUMN applies_to_roles TO applies_to_profiles;

ALTER TABLE bear_skills_manifest
    ADD CONSTRAINT bear_skills_manifest_applies_to_profiles_check
    CHECK (applies_to_profiles <@ ARRAY['chat', 'pair', 'curate', 'work', 'watch']::TEXT[]);
