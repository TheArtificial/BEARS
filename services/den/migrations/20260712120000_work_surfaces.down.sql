DROP INDEX IF EXISTS idx_bear_jobs_work_surface_id;
ALTER TABLE bear_jobs DROP COLUMN IF EXISTS work_surface_id;
DROP TABLE IF EXISTS work_surface_bears;
DROP TABLE IF EXISTS work_surface_managers;
DROP TABLE IF EXISTS work_surfaces;
