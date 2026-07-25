-- Empty is intentional: existing and newly created surfaces have no external
-- egress until their owner explicitly saves an allowed hostname.
ALTER TABLE work_surfaces
    ADD COLUMN allowed_outbound_hosts TEXT[] NOT NULL DEFAULT '{}';
