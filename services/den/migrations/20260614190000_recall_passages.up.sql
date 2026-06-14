-- Derived recall passage registry (ADR-0038 §3): Postgres metadata only. Vectors live in
-- Qdrant; canonical text lives in per-Bear SQLite. This table enables idempotent upsert,
-- delete-on-supersede, and reindex/reconcile progress without treating Qdrant as a source
-- of truth.
CREATE TABLE IF NOT EXISTS recall_passages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bear_id UUID NOT NULL REFERENCES bears(id) ON DELETE CASCADE,
    -- Canonical SQLite memory record id (TEXT UUID in the per-Bear store).
    memory_id TEXT NOT NULL,
    logical_path TEXT NULL,
    chunk_index INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    embedding_standard TEXT NOT NULL,
    source_class TEXT NOT NULL DEFAULT 'bear_memory',
    -- Deterministic Qdrant point id for this (bear, memory, chunk, standard) tuple.
    point_id TEXT NOT NULL,
    indexed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ NULL,
    CONSTRAINT uq_recall_passages_chunk UNIQUE (bear_id, memory_id, chunk_index, embedding_standard)
);

CREATE INDEX IF NOT EXISTS idx_recall_passages_bear_memory
    ON recall_passages (bear_id, memory_id);

CREATE INDEX IF NOT EXISTS idx_recall_passages_bear_standard_live
    ON recall_passages (bear_id, embedding_standard)
    WHERE deleted_at IS NULL;

COMMENT ON TABLE recall_passages IS 'Derived recall index registry (ADR-0038): metadata for Qdrant-backed Bear memory passages; not a source of truth.';
