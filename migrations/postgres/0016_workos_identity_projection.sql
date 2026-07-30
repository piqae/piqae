ALTER TABLE users
    ADD COLUMN identity_status text NOT NULL DEFAULT 'active'
        CHECK (identity_status IN ('active', 'inactive')),
    ADD COLUMN identity_updated_at timestamptz;

ALTER TABLE workspaces
    ADD COLUMN identity_updated_at timestamptz;

COMMENT ON COLUMN users.identity_status IS
    'Identity-provider lifecycle state. Inactive users retain historical tenant references but cannot authenticate through a projected membership.';

COMMENT ON COLUMN users.identity_updated_at IS
    'Latest identity-provider event timestamp applied to the user projection.';

COMMENT ON COLUMN workspaces.identity_updated_at IS
    'Latest identity-provider event timestamp applied to the workspace projection.';
