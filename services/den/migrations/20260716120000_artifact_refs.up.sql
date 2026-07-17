CREATE TABLE IF NOT EXISTS artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_ref TEXT NOT NULL UNIQUE,
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    created_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    owner_profile TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NULL,
    summary TEXT NULL,
    content_type TEXT NULL,
    storage_kind TEXT NOT NULL,
    storage_key TEXT NULL,
    content_bytes BIGINT NULL,
    content_sha256 TEXT NULL,
    lifecycle TEXT NOT NULL DEFAULT 'pending',
    visibility TEXT NOT NULL DEFAULT 'same_user',
    provenance JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    expires_at TIMESTAMPTZ NULL,
    finalized_at TIMESTAMPTZ NULL,
    deleted_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (artifact_ref ~ '^artifact_[0-9a-f]{32}$'),
    CHECK (owner_profile IN ('chat', 'pair', 'curate', 'work', 'watch')),
    CHECK (kind <> ''),
    CHECK (storage_kind IN ('db_text', 'garage_artifacts')),
    CHECK (lifecycle IN ('pending', 'finalized', 'deleted', 'expired')),
    CHECK (visibility IN ('private_to_profile', 'same_user', 'bear_visible', 'handoff_requested')),
    CHECK (storage_key IS NULL OR storage_key <> ''),
    CHECK (content_bytes IS NULL OR content_bytes >= 0),
    CHECK (content_sha256 IS NULL OR content_sha256 ~ '^[0-9a-f]{64}$'),
    CHECK (jsonb_typeof(provenance) = 'object'),
    CHECK (jsonb_typeof(metadata) = 'object'),
    CHECK (lifecycle <> 'finalized' OR finalized_at IS NOT NULL),
    CHECK (lifecycle <> 'deleted' OR deleted_at IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_artifacts_bear_ref
    ON artifacts (bear_id, artifact_ref);

CREATE INDEX IF NOT EXISTS idx_artifacts_bear_created
    ON artifacts (bear_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_artifacts_gc_candidates
    ON artifacts (expires_at)
    WHERE expires_at IS NOT NULL AND lifecycle IN ('finalized', 'expired');

CREATE TABLE IF NOT EXISTS artifact_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id UUID NOT NULL REFERENCES artifacts (id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    role TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (target_kind <> ''),
    CHECK (target_id <> ''),
    CHECK (role <> ''),
    CHECK (jsonb_typeof(metadata) = 'object'),
    UNIQUE (artifact_id, target_kind, target_id, role)
);

CREATE INDEX IF NOT EXISTS idx_artifact_links_target
    ON artifact_links (target_kind, target_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_artifact_links_artifact
    ON artifact_links (artifact_id, created_at DESC);
