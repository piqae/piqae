ALTER TABLE workspaces
    ADD COLUMN slug text,
    ADD COLUMN status text NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'suspended', 'cancelled')),
    ADD COLUMN updated_at timestamptz NOT NULL DEFAULT now();

UPDATE workspaces
SET slug = lower(regexp_replace(name, '[^a-zA-Z0-9]+', '-', 'g'))
           || '-' || lower(substr(replace(id, '_', ''), greatest(length(replace(id, '_', '')) - 5, 1)))
WHERE slug IS NULL;

ALTER TABLE workspaces ALTER COLUMN slug SET NOT NULL;
CREATE UNIQUE INDEX workspaces_slug_idx ON workspaces (slug);

ALTER TABLE workspace_members
    ADD COLUMN workos_membership_id text,
    ADD COLUMN status text NOT NULL DEFAULT 'active'
        CHECK (status IN ('pending', 'active', 'inactive')),
    ADD COLUMN role_updated_at timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN updated_at timestamptz NOT NULL DEFAULT now();

CREATE UNIQUE INDEX workspace_members_workos_id_idx
    ON workspace_members (workos_membership_id)
    WHERE workos_membership_id IS NOT NULL;

CREATE TABLE identity_webhook_receipts (
    provider text NOT NULL,
    event_id text NOT NULL,
    event_type text NOT NULL,
    payload_sha256 text NOT NULL,
    processed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, event_id)
);

CREATE TABLE local_owner_credentials (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    key_hash text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    rotated_at timestamptz,
    revoked_at timestamptz
);

CREATE UNIQUE INDEX local_owner_one_active_per_workspace_idx
    ON local_owner_credentials (workspace_id)
    WHERE revoked_at IS NULL;

CREATE TABLE local_owner_sessions (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    credential_id text NOT NULL REFERENCES local_owner_credentials(id) ON DELETE CASCADE,
    token_hash text NOT NULL UNIQUE,
    expires_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX local_owner_sessions_active_idx
    ON local_owner_sessions (workspace_id, expires_at)
    WHERE revoked_at IS NULL;
