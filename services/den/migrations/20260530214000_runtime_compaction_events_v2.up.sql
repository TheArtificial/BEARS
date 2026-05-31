ALTER TABLE runtime_compaction_events
    ADD COLUMN IF NOT EXISTS event_hash TEXT;

UPDATE runtime_compaction_events
SET event_hash = md5(
    concat_ws(
        '|',
        conversation_id,
        trigger,
        policy_version,
        status,
        COALESCE(boundary::text, ''),
        COALESCE(source_group_start::text, ''),
        COALESCE(source_group_end::text, ''),
        COALESCE(artifact::text, ''),
        COALESCE(diagnostic, '')
    )
)
WHERE event_hash IS NULL;

ALTER TABLE runtime_compaction_events
    ALTER COLUMN event_hash SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_runtime_compaction_events_dedupe
    ON runtime_compaction_events (conversation_id, event_hash);
