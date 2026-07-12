-- Sandbox image catalog managed in Den: name -> container image reference.
-- The catalog remains the dispatch trust boundary — Den sends catalog names
-- to the sandbox provider, which resolves references from its synced copy.
-- Replaces the static data/sandbox-roots.json image list.

CREATE TABLE sandbox_catalog_images (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    image_ref TEXT NOT NULL CHECK (btrim(image_ref) <> ''),
    description TEXT NULL,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    -- Nullable so migrations can seed rows without a user.
    created_by_user_id INTEGER NULL REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- At most one default image.
CREATE UNIQUE INDEX idx_sandbox_catalog_one_default
    ON sandbox_catalog_images ((TRUE)) WHERE is_default;

INSERT INTO sandbox_catalog_images (name, image_ref, description, is_default) VALUES
    ('base', 'bears/sandbox:latest', 'Debian slim + armature, git, curl, ripgrep', TRUE),
    ('rust', 'bears/sandbox-rust:latest', 'base + rustup stable toolchain, clippy, rustfmt', FALSE),
    ('node', 'bears/sandbox-node:latest', 'base + Node.js 22 (11ty-ready)', FALSE),
    ('godot', 'bears/sandbox-godot:latest', 'base + Godot editor binary and export templates (godot --headless)', FALSE);
