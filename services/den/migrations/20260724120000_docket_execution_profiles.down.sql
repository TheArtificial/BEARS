ALTER TABLE docket_turn_attempts
    DROP COLUMN cost_microusd,
    DROP COLUMN latency_ms,
    DROP COLUMN profile_provenance,
    DROP COLUMN resolved_profile;
