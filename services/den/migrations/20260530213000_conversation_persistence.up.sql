CREATE TABLE IF NOT EXISTS conversations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bear_id UUID NOT NULL REFERENCES bears (id) ON DELETE CASCADE,
    created_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    source_acp_session_id TEXT NULL,
    external_conversation_id TEXT NULL,
    current_title TEXT NULL,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'archived', 'deleted', 'migrated')),
    archive_state TEXT NOT NULL DEFAULT 'live'
        CHECK (archive_state IN ('live', 'archived', 'restored', 'superseded')),
    provider_binding JSONB NOT NULL DEFAULT '{}'::JSONB,
    workspace_context JSONB NOT NULL DEFAULT '{}'::JSONB,
    restored_from_conversation_id UUID NULL REFERENCES conversations (id) ON DELETE SET NULL,
    archived_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (jsonb_typeof(provider_binding) = 'object'),
    CHECK (jsonb_typeof(workspace_context) = 'object')
);

CREATE INDEX IF NOT EXISTS idx_conversations_bear_updated
    ON conversations (bear_id, updated_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_conversations_bear_external_conversation
    ON conversations (bear_id, external_conversation_id)
    WHERE external_conversation_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS conversation_messages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    sequence_no BIGINT NOT NULL,
    message_type TEXT NOT NULL
        CHECK (message_type IN (
            'user',
            'assistant',
            'system',
            'developer',
            'tool_call',
            'tool_result',
            'workflow_event',
            'compaction_marker'
        )),
    role TEXT NULL,
    visibility TEXT NOT NULL DEFAULT 'default'
        CHECK (visibility IN ('default', 'hidden_from_user', 'admin_only', 'diagnostic_only')),
    content_text TEXT NOT NULL DEFAULT '',
    content_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    tool_name TEXT NULL,
    tool_call_id TEXT NULL,
    source_event_id TEXT NULL,
    provider_message_id TEXT NULL,
    created_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    created_by_role TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    redacted_at TIMESTAMPTZ NULL,
    redaction_reason TEXT NULL,
    CHECK (jsonb_typeof(content_json) = 'object')
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_conversation_messages_sequence
    ON conversation_messages (conversation_id, sequence_no);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_created
    ON conversation_messages (conversation_id, created_at DESC);

CREATE TABLE IF NOT EXISTS conversation_compaction_artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    artifact_kind TEXT NOT NULL
        CHECK (artifact_kind IN ('iterative_summary', 'semantic_window', 'archive_summary', 'migration_summary')),
    policy_version TEXT NOT NULL,
    trigger TEXT NOT NULL,
    source_message_start_seq BIGINT NOT NULL,
    source_message_end_seq BIGINT NOT NULL,
    source_group_start INTEGER NULL,
    source_group_end INTEGER NULL,
    artifact_json JSONB NOT NULL,
    superseded_by UUID NULL REFERENCES conversation_compaction_artifacts (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (jsonb_typeof(artifact_json) = 'object'),
    CHECK (source_message_end_seq >= source_message_start_seq)
);

CREATE INDEX IF NOT EXISTS idx_conversation_compaction_artifacts_conversation
    ON conversation_compaction_artifacts (conversation_id, created_at DESC);

CREATE TABLE IF NOT EXISTS conversation_archives (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    archive_version INTEGER NOT NULL,
    archive_reason TEXT NOT NULL,
    summary_artifact_id UUID NULL REFERENCES conversation_compaction_artifacts (id) ON DELETE SET NULL,
    archive_manifest JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    restored_at TIMESTAMPTZ NULL,
    restored_to_conversation_id UUID NULL REFERENCES conversations (id) ON DELETE SET NULL,
    CHECK (archive_version > 0),
    CHECK (jsonb_typeof(archive_manifest) = 'object')
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_conversation_archives_version
    ON conversation_archives (conversation_id, archive_version);

COMMENT ON TABLE conversations IS 'Den-owned canonical logical conversations replacing provider-owned transcript identity.';
COMMENT ON TABLE conversation_messages IS 'Immutable ordered transcript log for Den-owned conversations.';
COMMENT ON TABLE conversation_compaction_artifacts IS 'First-class persisted conversation compaction/summary artifacts with source span provenance.';
COMMENT ON TABLE conversation_archives IS 'Archive projections and restore lineage for Den-owned conversations.';
