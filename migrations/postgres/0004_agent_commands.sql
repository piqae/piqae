CREATE TABLE agent_commands (
    cursor bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    agent_id text NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    command jsonb NOT NULL,
    delivered_at timestamptz,
    acknowledged_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX agent_commands_pending_idx
    ON agent_commands (agent_id, cursor)
    WHERE acknowledged_at IS NULL;
