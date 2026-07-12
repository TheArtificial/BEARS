-- Remove retired external-runtime compatibility columns from live schema.
-- Historical migrations retain original names for replay/checksum stability.

ALTER TABLE bears
    DROP COLUMN IF EXISTS letta_agent_type,
    DROP COLUMN IF EXISTS letta_tool_ids,
    DROP COLUMN IF EXISTS memfs_repo_path;

DROP INDEX IF EXISTS idx_bear_profile_bindings_letta_agent_id_unique;

ALTER TABLE bear_profile_bindings
    DROP COLUMN IF EXISTS letta_agent_id;
