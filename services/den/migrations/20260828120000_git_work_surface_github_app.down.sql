ALTER TABLE git_work_surface_details
    DROP CONSTRAINT IF EXISTS git_work_surface_details_github_app_write_requires_installation,
    DROP COLUMN IF EXISTS github_app_write_enabled,
    DROP COLUMN IF EXISTS github_app_installation_id;