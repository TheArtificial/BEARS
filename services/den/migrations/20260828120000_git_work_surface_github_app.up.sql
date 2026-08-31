-- A surface may opt into the provider's deployment-scoped GitHub App. The
-- installation id is public configuration, not a credential; the App private
-- key stays exclusively in the sandbox provider's secret store.
ALTER TABLE git_work_surface_details
    ADD COLUMN github_app_installation_id BIGINT NULL
        CHECK (github_app_installation_id > 0),
    ADD COLUMN github_app_write_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD CONSTRAINT git_work_surface_details_github_app_write_requires_installation
        CHECK (NOT github_app_write_enabled OR github_app_installation_id IS NOT NULL);