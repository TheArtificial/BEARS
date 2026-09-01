DROP TABLE IF EXISTS cabinet_source_links;

ALTER TABLE cabinet_items
    DROP CONSTRAINT IF EXISTS fk_cabinet_items_current_version;

DROP TABLE IF EXISTS cabinet_item_versions;

DROP TABLE IF EXISTS cabinet_items;

ALTER TABLE bears
    DROP COLUMN IF EXISTS cabinet_enabled;
