ALTER TABLE bear_bifrost_virtual_keys
    ADD COLUMN IF NOT EXISTS virtual_key_value_encrypted TEXT NULL;
