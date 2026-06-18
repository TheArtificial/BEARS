-- ADR-0036: operational profile vocabulary (not membership role).

ALTER TABLE bear_memory_proposals RENAME COLUMN source_role TO source_profile;
ALTER TABLE bear_memory_proposals RENAME COLUMN reviewer_role TO reviewer_profile;

ALTER TABLE bear_memory_proposals DROP CONSTRAINT IF EXISTS bear_memory_proposals_source_role_check;
ALTER TABLE bear_memory_proposals DROP CONSTRAINT IF EXISTS bear_memory_proposals_reviewer_role_check;

ALTER TABLE bear_memory_proposals
    ADD CONSTRAINT bear_memory_proposals_source_profile_check
    CHECK (source_profile IN ('chat', 'pair', 'curate', 'work', 'watch'));

ALTER TABLE bear_memory_proposals
    ADD CONSTRAINT bear_memory_proposals_reviewer_profile_check
    CHECK (reviewer_profile IS NULL OR reviewer_profile IN ('chat', 'pair', 'curate', 'work', 'watch'));

DROP INDEX IF EXISTS idx_bear_memory_proposals_bear_source_role_created;
CREATE INDEX IF NOT EXISTS idx_bear_memory_proposals_bear_source_profile_created
    ON bear_memory_proposals (bear_id, source_profile, created_at DESC);

ALTER TABLE bear_work_plans RENAME COLUMN owner_role TO owner_profile;

ALTER TABLE bear_work_plans DROP CONSTRAINT IF EXISTS bear_work_plans_owner_role_check;

ALTER TABLE bear_work_plans
    ADD CONSTRAINT bear_work_plans_owner_profile_check
    CHECK (owner_profile IN ('chat', 'pair', 'curate', 'work', 'watch'));

ALTER TABLE bear_work_plans DROP CONSTRAINT IF EXISTS bear_work_plans_visibility_check;

-- Migrate the legacy value BEFORE enforcing the new vocabulary, otherwise the
-- ADD CONSTRAINT fails on any database that still holds pre-vocabulary rows.
UPDATE bear_work_plans SET visibility = 'private_to_profile' WHERE visibility = 'private_to_role';

ALTER TABLE bear_work_plans
    ADD CONSTRAINT bear_work_plans_visibility_check
    CHECK (visibility IN ('private_to_profile', 'same_user', 'bear_visible', 'handoff_requested'));

DROP INDEX IF EXISTS idx_bear_work_plans_owner;
CREATE INDEX IF NOT EXISTS idx_bear_work_plans_owner
    ON bear_work_plans (bear_id, owner_profile, updated_at DESC);

UPDATE prompt_memory_blocks SET scope = 'profile_local' WHERE scope = 'role_local';
UPDATE prompt_memory_blocks SET block_type = 'profile_guidance' WHERE block_type = 'role_guidance';

ALTER TABLE prompt_memory_blocks DROP CONSTRAINT IF EXISTS prompt_memory_blocks_scope_check;
ALTER TABLE prompt_memory_blocks
    ADD CONSTRAINT prompt_memory_blocks_scope_check
    CHECK (scope IN ('bear_wide', 'profile_local', 'work_surface', 'session'));

ALTER TABLE prompt_memory_blocks DROP CONSTRAINT IF EXISTS prompt_memory_blocks_block_type_check;
ALTER TABLE prompt_memory_blocks
    ADD CONSTRAINT prompt_memory_blocks_block_type_check
    CHECK (block_type IN ('profile_guidance', 'work_surface_context', 'session_focus', 'user_instruction'));
