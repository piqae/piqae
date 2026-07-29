CREATE TABLE platform_service_accounts (
    id text PRIMARY KEY,
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    secret_hash text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    revoked_at timestamptz
);

ALTER TABLE environments
    ADD CONSTRAINT environments_workspace_id_id_key UNIQUE (workspace_id, id);

CREATE TABLE platform_workspace_grants (
    id text PRIMARY KEY,
    service_account_id text NOT NULL
        REFERENCES platform_service_accounts(id) ON DELETE CASCADE,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL,
    scopes text[] NOT NULL CHECK (
        cardinality(scopes) > 0
        AND scopes <@ ARRAY[
            'api_keys_read', 'api_keys_write', 'agents_read', 'agents_write',
            'printers_read', 'printers_write', 'jobs_read', 'jobs_write',
            'webhooks_read', 'webhooks_write', 'usage_read', 'audit_read'
        ]::text[]
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz,
    revoked_at timestamptz,
    FOREIGN KEY (workspace_id, environment_id)
        REFERENCES environments(workspace_id, id) ON DELETE CASCADE,
    UNIQUE (service_account_id, workspace_id, environment_id)
);

CREATE INDEX platform_workspace_grants_active_idx
    ON platform_workspace_grants (service_account_id, workspace_id, environment_id)
    WHERE revoked_at IS NULL;
