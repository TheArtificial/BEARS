ALTER TABLE prompt_memory_blocks DROP CONSTRAINT IF EXISTS prompt_memory_blocks_block_type_check;
ALTER TABLE prompt_memory_blocks
    ADD CONSTRAINT prompt_memory_blocks_block_type_check
    CHECK (block_type IN ('role_guidance', 'work_surface_context', 'session_focus', 'user_instruction'));

UPDATE prompt_memory_blocks SET block_type = 'role_guidance' WHERE block_type = 'profile_guidance';

ALTER TABLE prompt_memory_blocks DROP CONSTRAINT IF EXISTS prompt_memory_blocks_scope_check;
ALTER TABLE prompt_memory_blocks
    ADD CONSTRAINT prompt_memory_blocks_scope_check
    CHECK (scope IN ('bear_wide', 'role_local', 'work_surface', 'session'));

UPDATE prompt_memory_blocks SET scope = 'role_local' WHERE scope = 'profile_local';

DROP INDEX IF EXISTS idx_bear_work_plans_owner;
CREATE INDEX IF NOT EXISTS idx_bear_work_plans_owner
    ON bear_work_plans (bear_id, owner_role, updated_at DESC);

UPDATE bear_work_plans SET visibility = 'private_to_role' WHERE visibility = 'private_to_profile';

ALTER TABLE bear_work_plans DROP CONSTRAINT IF EXISTS bear_work_plans_visibility_check;
ALTER TABLE bear_work_plans
    ADD CONSTRAINT bear_work_plans_visibility_check
    CHECK (visibility IN ('private_to_role', 'same_user', 'bear_visible', 'handoff_requested'));

ALTER TABLE bear_work_plans DROP CONSTRAINT IF EXISTS bear_work_plans_owner_profile_check;
ALTER TABLE bear_work_plans RENAME COLUMN owner_profile TO owner_role;
ALTER TABLE bear_work_plans
    ADD CONSTRAINT bear_work_plans_owner_role_check
    CHECK (owner_role IN ('chat', 'pair', 'curate', 'work', 'watch'));

DROP INDEX IF EXISTS idx_bear_memory_proposals_bear_source_profile_created;
CREATE INDEX IF NOT EXISTS idx_bear_memory_proposals_bear_source_role_created
    ON bear_memory_proposals (bear_id, source_role, created_at DESC);

ALTER TABLE bear_memory_proposals DROP CONSTRAINT IF EXISTS bear_memory_proposals_reviewer_profile_check;
ALTER TABLE bear_memory_proposals DROP CONSTRAINT IF EXISTS bear_memory_proposals_source_profile_check;
ALTER TABLE bear_memory_proposals RENAME COLUMN reviewer_profile TO reviewer_role;
ALTER TABLE bear_memory_proposals RENAME COLUMN source_profile TO source_role;

ALTER TABLE bear_memory_proposals
    ADD CONSTRAINT bear_memory_proposals_source_role_check
    CHECK (source_role IN ('chat', 'pair', 'curate', 'work', 'watch'));
ALTER TABLE bear_memory_proposals
    ADD CONSTRAINT bear_memory_proposals_reviewer_role_check
    CHECK (reviewer_role IS NULL OR reviewer_role IN ('chat', 'pair', 'curate', 'work', 'watch'));
