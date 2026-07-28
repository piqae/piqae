CREATE TABLE uploads (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    object_key text NOT NULL UNIQUE,
    media_type text NOT NULL CHECK (media_type IN ('application/pdf', 'application/octet-stream')),
    expected_sha256 text NOT NULL,
    expected_bytes bigint NOT NULL CHECK (expected_bytes >= 0),
    state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'complete', 'expired')),
    expires_at timestamptz NOT NULL,
    completed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX uploads_expiry_idx ON uploads (expires_at) WHERE completed_at IS NULL;
