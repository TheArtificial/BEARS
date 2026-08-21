-- Evidence that satisfies a checkpoint directive must be durable and replayable.
ALTER TABLE docket_checkpoint_directives
    ADD COLUMN acknowledged_artifact_ref TEXT NULL REFERENCES artifacts (artifact_ref);

CREATE UNIQUE INDEX docket_checkpoint_directives_acknowledged_artifact_idx
    ON docket_checkpoint_directives (acknowledged_artifact_ref)
    WHERE acknowledged_artifact_ref IS NOT NULL;
