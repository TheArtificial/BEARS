CREATE TABLE IF NOT EXISTS bear_observations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bear_id UUID NOT NULL REFERENCES bears(id) ON DELETE CASCADE,
    observation_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    salience TEXT NOT NULL DEFAULT 'normal' CHECK (salience IN (
        'low',
        'normal',
        'high',
        'critical'
    )),
    payload_ref TEXT NULL,
    source JSONB NOT NULL DEFAULT '{}',
    logical_path TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending_review' CHECK (status IN (
        'pending_review',
        'review_queued',
        'reviewed',
        'dismissed'
    )),
    proposal_id UUID NULL REFERENCES bear_memory_proposals(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reviewed_at TIMESTAMPTZ NULL,
    CONSTRAINT uq_bear_observations_bear_observation_id UNIQUE (bear_id, observation_id)
);

CREATE INDEX IF NOT EXISTS idx_bear_observations_bear_status_created
    ON bear_observations (bear_id, status, created_at DESC);

COMMENT ON TABLE bear_observations IS 'Den-owned watch observations pending curate review.';
