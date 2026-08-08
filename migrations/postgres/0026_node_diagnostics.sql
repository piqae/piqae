CREATE TABLE node_diagnostics (
    request_id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL,
    agent_id text NOT NULL,
    state text NOT NULL CHECK (state IN ('requested', 'complete', 'failed')),
    report jsonb,
    requested_at timestamptz NOT NULL DEFAULT now(),
    received_at timestamptz,
    expires_at timestamptz NOT NULL DEFAULT now() + interval '14 days',
    CHECK (octet_length(COALESCE(report::text, '')) <= 16384),
    FOREIGN KEY (environment_id, workspace_id)
        REFERENCES environments(id, workspace_id) ON DELETE CASCADE,
    FOREIGN KEY (agent_id, workspace_id, environment_id)
        REFERENCES agents(id, workspace_id, environment_id) ON DELETE CASCADE
);

CREATE INDEX node_diagnostics_tenant_node_requested_idx
    ON node_diagnostics (workspace_id, environment_id, agent_id, requested_at DESC);

CREATE INDEX node_diagnostics_expiry_idx ON node_diagnostics (expires_at);
