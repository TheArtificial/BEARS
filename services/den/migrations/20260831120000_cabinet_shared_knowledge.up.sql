-- Cabinet Phase 1: shared, human-editable knowledge behind the Den facade.
-- Contract: docs/architecture/cabinet-contract.md
-- Plan: docs/roadmap/CABINET_IMPLEMENTATION_PLAN.md

-- Per-Bear Cabinet gate, following the bears.work_enabled mold. Default true
-- matches the open-wiki default; operators disable per Bear.
ALTER TABLE bears
    ADD COLUMN IF NOT EXISTS cabinet_enabled BOOLEAN NOT NULL DEFAULT true;

COMMENT ON COLUMN bears.cabinet_enabled IS
    'Enables Cabinet shared-knowledge tools and facade access for this Bear.';

CREATE TABLE IF NOT EXISTS cabinet_items (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cabinet_ref TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    lifecycle TEXT NOT NULL DEFAULT 'active',
    -- Phase 2 scope bindings; columns exist so the contract shape is stable,
    -- but Phase 1 rejects requests that set them.
    collection_ref TEXT NULL,
    mission_ref TEXT NULL,
    current_version_id UUID NULL,
    created_by JSONB NOT NULL,
    created_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    created_by_bear_id UUID NULL REFERENCES bears (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (cabinet_ref ~ '^cabinet_item_[0-9a-f]{32}$'),
    CHECK (kind IN ('document')),
    CHECK (btrim(title) <> ''),
    CHECK (lifecycle IN ('active', 'archived', 'deleted')),
    CHECK (collection_ref IS NULL OR collection_ref ~ '^cabinet_collection_[0-9a-f]{32}$'),
    CHECK (mission_ref IS NULL OR mission_ref ~ '^mission_[0-9a-f]{32}$'),
    CHECK (jsonb_typeof(created_by) = 'object')
);

COMMENT ON TABLE cabinet_items IS
    'Cabinet shared-knowledge items (contract: docs/architecture/cabinet-contract.md). Deletion is a tombstone; versions remain citable.';
COMMENT ON COLUMN cabinet_items.created_by IS
    'Verbatim ActorScope provenance; the sibling *_user_id/*_bear_id columns are denormalized query keys.';

CREATE TABLE IF NOT EXISTS cabinet_item_versions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    version_ref TEXT NOT NULL UNIQUE,
    item_id UUID NOT NULL REFERENCES cabinet_items (id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    base_version_ref TEXT NULL,
    review TEXT NOT NULL DEFAULT 'none',
    authored_by JSONB NOT NULL,
    authored_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    authored_by_bear_id UUID NULL REFERENCES bears (id) ON DELETE SET NULL,
    authored_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (item_id, revision),
    CHECK (version_ref ~ '^cabinet_version_[0-9a-f]{32}$'),
    CHECK (revision >= 1),
    CHECK ((revision = 1) = (base_version_ref IS NULL)),
    CHECK (base_version_ref IS NULL OR base_version_ref ~ '^cabinet_version_[0-9a-f]{32}$'),
    CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    CHECK (review IN ('none', 'pending', 'approved', 'rejected')),
    CHECK (jsonb_typeof(authored_by) = 'object')
);

COMMENT ON TABLE cabinet_item_versions IS
    'Immutable Cabinet item versions: the citation unit and revision history. Rows are never updated after insert.';

ALTER TABLE cabinet_items
    ADD CONSTRAINT fk_cabinet_items_current_version
    FOREIGN KEY (current_version_id) REFERENCES cabinet_item_versions (id);

CREATE INDEX IF NOT EXISTS idx_cabinet_items_updated
    ON cabinet_items (updated_at DESC)
    WHERE lifecycle <> 'deleted';

CREATE INDEX IF NOT EXISTS idx_cabinet_item_versions_item
    ON cabinet_item_versions (item_id, revision DESC);

CREATE TABLE IF NOT EXISTS cabinet_source_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_ref TEXT NOT NULL UNIQUE,
    item_id UUID NOT NULL REFERENCES cabinet_items (id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL,
    locator TEXT NOT NULL,
    role TEXT NOT NULL,
    created_by JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (item_id, source_kind, locator, role),
    CHECK (source_ref ~ '^cabinet_source_[0-9a-f]{32}$'),
    CHECK (source_kind IN ('url', 'offline', 'artifact', 'conversation', 'external_record')),
    CHECK (btrim(locator) <> ''),
    CHECK (role IN ('origin', 'citation', 'related')),
    CHECK (jsonb_typeof(created_by) = 'object')
);

COMMENT ON TABLE cabinet_source_links IS
    'Provenance from Cabinet items to material outside Cabinet. Links are provenance, not content.';

CREATE INDEX IF NOT EXISTS idx_cabinet_source_links_item
    ON cabinet_source_links (item_id, created_at DESC);
