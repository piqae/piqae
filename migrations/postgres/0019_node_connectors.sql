-- Separate the durable physical installation from each tenant-visible agent
-- projection. Existing agents remain protocol-compatible connector endpoints.
CREATE TABLE node_installations (
    id text PRIMARY KEY,
    installation_key text NOT NULL UNIQUE,
    public_key bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE environments
    ADD CONSTRAINT environments_id_workspace_unique UNIQUE (id, workspace_id);

CREATE TABLE node_connectors (
    id text PRIMARY KEY,
    installation_id text NOT NULL REFERENCES node_installations(id) ON DELETE CASCADE,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL,
    agent_id text NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    permissions jsonb NOT NULL DEFAULT '{"printers":"all","print_jobs":"create_and_monitor"}'::jsonb,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, environment_id, agent_id),
    UNIQUE (installation_id, workspace_id, environment_id),
    FOREIGN KEY (environment_id, workspace_id)
        REFERENCES environments(id, workspace_id) ON DELETE CASCADE
);

CREATE INDEX node_connectors_tenant_active_idx
    ON node_connectors (workspace_id, environment_id, agent_id)
    WHERE revoked_at IS NULL;

-- Backfill one installation and connector for every existing tenant agent.
-- Legacy rows are intentionally backfilled one-to-one. Historical identifiers
-- are tenant-local and must not be used to merge tenants without a fresh,
-- proof-of-possession connection handshake.
INSERT INTO node_installations (id, installation_key, public_key)
SELECT 'ninst_' || id, 'legacy:' || id, public_key
FROM agents
WHERE public_key IS NOT NULL
ON CONFLICT (installation_key) DO NOTHING;

INSERT INTO node_connectors (id, installation_id, workspace_id, environment_id, agent_id)
SELECT 'ncon_' || agent.id, installation.id,
       agent.workspace_id, agent.environment_id, agent.id
FROM agents agent
JOIN node_installations installation
  ON installation.installation_key = 'legacy:' || agent.id
WHERE agent.public_key IS NOT NULL
ON CONFLICT (workspace_id, environment_id, agent_id) DO NOTHING;
