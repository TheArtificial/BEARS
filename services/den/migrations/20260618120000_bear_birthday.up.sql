ALTER TABLE bears
    ADD COLUMN IF NOT EXISTS birthday DATE NULL;

COMMENT ON COLUMN bears.birthday IS 'Bear identity birthday, distinct from row creation timestamp. Used for portable .bear export/import identity.';
