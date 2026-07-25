-- ADR-0056 Phase 2: supervisor-owned profile escalation and attempt attribution.

ALTER TABLE docket_turn_attempts
    ADD COLUMN resolved_profile TEXT NULL
        CHECK (resolved_profile IS NULL OR resolved_profile IN ('economy', 'balanced', 'advanced')),
    ADD COLUMN profile_provenance TEXT NOT NULL DEFAULT 'conversation_fallback'
        CHECK (profile_provenance IN ('task_difficulty', 'conversation_fallback', 'supervisor_escalation')),
    ADD COLUMN latency_ms BIGINT NULL CHECK (latency_ms IS NULL OR latency_ms >= 0),
    ADD COLUMN cost_microusd BIGINT NULL CHECK (cost_microusd IS NULL OR cost_microusd >= 0);

COMMENT ON COLUMN docket_turn_attempts.resolved_profile IS
    'Symbolic execution tier, never a provider or concrete model identifier.';
COMMENT ON COLUMN docket_turn_attempts.cost_microusd IS
    'Optional provider-reported cost attribution in millionths of a US dollar.';
