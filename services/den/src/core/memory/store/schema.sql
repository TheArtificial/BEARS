PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA busy_timeout=5000;

CREATE TABLE IF NOT EXISTS bear_sequence (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    next_sequence INTEGER NOT NULL DEFAULT 1
);
INSERT OR IGNORE INTO bear_sequence (id, next_sequence) VALUES (1, 1);

CREATE TABLE IF NOT EXISTS memory_records (
    memory_id TEXT PRIMARY KEY,
    bear_id TEXT NOT NULL,
    sequence_no INTEGER NOT NULL,
    scope_type TEXT NOT NULL CHECK (scope_type IN ('profile_local', 'shared')),
    scope_profile TEXT NULL,
    kind TEXT NOT NULL,
    entity_ref TEXT NULL,
    author_profile TEXT NOT NULL,
    author_agent_id TEXT NULL,
    created_at TEXT NOT NULL,
    content_text TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    supersedes_memory_id TEXT NULL,
    visibility TEXT NOT NULL DEFAULT 'normal',
    logical_path TEXT NULL,
    work_surface_ref TEXT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_records_bear_sequence
    ON memory_records (bear_id, sequence_no);
CREATE INDEX IF NOT EXISTS idx_memory_records_logical_path
    ON memory_records (bear_id, logical_path);

CREATE TABLE IF NOT EXISTS memory_links (
    link_id TEXT PRIMARY KEY,
    bear_id TEXT NOT NULL,
    sequence_no INTEGER NOT NULL,
    src_memory_id TEXT NOT NULL,
    dst_ref_type TEXT NOT NULL,
    dst_ref TEXT NOT NULL,
    link_type TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_promotions (
    promotion_id TEXT PRIMARY KEY,
    bear_id TEXT NOT NULL,
    sequence_no INTEGER NOT NULL,
    source_memory_id TEXT NOT NULL,
    target_memory_id TEXT NULL,
    review_agent_id TEXT NULL,
    action TEXT NOT NULL,
    created_at TEXT NOT NULL,
    notes TEXT NULL
);

CREATE TABLE IF NOT EXISTS memory_proposals (
    proposal_id TEXT PRIMARY KEY,
    bear_id TEXT NOT NULL,
    sequence_no INTEGER NOT NULL,
    source_memory_id TEXT NULL,
    suggested_action TEXT NOT NULL,
    sensitivity TEXT NOT NULL DEFAULT 'normal',
    requires_human INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending',
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    reviewed_at TEXT NULL
);

CREATE TABLE IF NOT EXISTS memory_observations (
    observation_id TEXT PRIMARY KEY,
    bear_id TEXT NOT NULL,
    sequence_no INTEGER NOT NULL,
    summary TEXT NOT NULL,
    salience TEXT NOT NULL DEFAULT 'normal',
    payload_ref TEXT NULL,
    source_json TEXT NOT NULL DEFAULT '{}',
    logical_path TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending_review',
    proposal_id TEXT NULL,
    created_at TEXT NOT NULL,
    reviewed_at TEXT NULL
);

CREATE TABLE IF NOT EXISTS reflection_run_outcomes (
    run_id TEXT PRIMARY KEY,
    bear_id TEXT NOT NULL,
    sequence_no INTEGER NOT NULL,
    lane TEXT NOT NULL,
    trigger TEXT NOT NULL,
    status TEXT NOT NULL,
    input_summary TEXT NULL,
    output_summary TEXT NULL,
    proposal_ids_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    completed_at TEXT NULL
);
