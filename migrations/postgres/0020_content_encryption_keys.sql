ALTER TABLE agents
    ADD CONSTRAINT agents_tenant_identity_unique UNIQUE (id, workspace_id, environment_id);

CREATE TABLE node_content_encryption_keys (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    agent_id text NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    key_id text NOT NULL,
    algorithm text NOT NULL CHECK (algorithm = 'RSA-OAEP-256'),
    public_key_spki text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    PRIMARY KEY (workspace_id, environment_id, agent_id, key_id),
    FOREIGN KEY (agent_id, workspace_id, environment_id)
        REFERENCES agents(id, workspace_id, environment_id)
        ON DELETE CASCADE,
    CHECK (length(key_id) BETWEEN 1 AND 255),
    CHECK (length(public_key_spki) BETWEEN 128 AND 4096)
);

CREATE UNIQUE INDEX node_content_encryption_keys_active_idx
    ON node_content_encryption_keys (workspace_id, environment_id, agent_id)
    WHERE revoked_at IS NULL;

CREATE INDEX node_content_encryption_keys_lookup_idx
    ON node_content_encryption_keys (workspace_id, environment_id, key_id)
    WHERE revoked_at IS NULL;
