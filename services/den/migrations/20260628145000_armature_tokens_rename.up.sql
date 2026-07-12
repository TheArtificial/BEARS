-- ADR-0043: tokens authorize armature/client access, not ACP as a Den concept.

DO $$
BEGIN
    IF to_regclass('public.armature_tokens') IS NULL
       AND to_regclass('public.acp_tokens') IS NOT NULL THEN
        ALTER TABLE acp_tokens RENAME TO armature_tokens;
    END IF;

    IF to_regclass('public.armature_token_bears') IS NULL
       AND to_regclass('public.acp_token_bears') IS NOT NULL THEN
        ALTER TABLE acp_token_bears RENAME TO armature_token_bears;
    END IF;
END $$;

DROP INDEX IF EXISTS idx_acp_tokens_user_id;
DROP INDEX IF EXISTS idx_acp_tokens_active_user;

CREATE INDEX IF NOT EXISTS idx_armature_tokens_user_id ON armature_tokens (user_id);
CREATE INDEX IF NOT EXISTS idx_armature_tokens_active_user ON armature_tokens (user_id, revoked_at, expires_at);

UPDATE armature_tokens
SET scopes = (
    SELECT jsonb_agg(
        CASE value
            WHEN 'acp:chat' THEN 'armature:chat'
            WHEN 'acp:tools' THEN 'armature:tools'
            ELSE value
        END
    )
    FROM jsonb_array_elements_text(scopes::jsonb) AS scope(value)
)
WHERE scopes::jsonb ?| ARRAY['acp:chat', 'acp:tools'];

COMMENT ON TABLE armature_tokens IS 'User-owned personal access tokens for armature adapters. Store only token hashes; raw tokens are shown once.';
COMMENT ON TABLE armature_token_bears IS 'Per-bear grants for armature tokens. User membership is still checked at request time.';
