CREATE TABLE device_authorizations (
    id text PRIMARY KEY,
    device_code_hash text NOT NULL UNIQUE,
    user_code_hash text NOT NULL UNIQUE,
    user_code_display text NOT NULL,
    device_public_key bytea NOT NULL CHECK (octet_length(device_public_key) = 32),
    installation_id text NOT NULL,
    proposed_name text NOT NULL,
    hostname text NOT NULL,
    platform text NOT NULL,
    architecture text NOT NULL,
    installation_mode text NOT NULL
        CHECK (installation_mode IN ('user', 'machine', 'local')),
    agent_version text NOT NULL,
    protocol_version integer NOT NULL CHECK (protocol_version > 0),
    state text NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'approved', 'denied', 'consumed', 'expired')),
    expires_at timestamptz NOT NULL,
    workspace_id text REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text REFERENCES environments(id) ON DELETE CASCADE,
    approved_by text,
    approved_at timestamptz,
    denied_at timestamptz,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (state = 'approved' AND workspace_id IS NOT NULL AND environment_id IS NOT NULL AND approved_at IS NOT NULL)
        OR state <> 'approved'
    )
);

CREATE INDEX device_authorizations_expiry_idx
    ON device_authorizations (expires_at)
    WHERE consumed_at IS NULL;
CREATE INDEX device_authorizations_pending_code_idx
    ON device_authorizations (user_code_hash, expires_at)
    WHERE state = 'pending';
