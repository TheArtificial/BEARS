DROP INDEX IF EXISTS idx_runtime_compaction_events_dedupe;
ALTER TABLE runtime_compaction_events DROP COLUMN IF EXISTS event_hash;
