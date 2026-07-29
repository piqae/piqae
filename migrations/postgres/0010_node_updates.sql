CREATE TABLE node_update_policies (
    node_id text PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    channel text NOT NULL DEFAULT 'stable'
        CHECK (channel IN ('stable', 'canary', 'pinned')),
    mode text NOT NULL DEFAULT 'prompt'
        CHECK (mode IN ('automatic', 'prompt', 'disabled')),
    pinned_version text,
    maintenance_window jsonb,
    desired_version text,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (channel <> 'pinned' OR pinned_version IS NOT NULL)
);

CREATE TABLE node_update_states (
    node_id text PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    current_version text NOT NULL,
    available_version text,
    state text NOT NULL DEFAULT 'idle',
    download_percent smallint CHECK (download_percent BETWEEN 0 AND 100),
    deferred_reason text,
    last_checked_at timestamptz,
    last_success_at timestamptz,
    last_error_code text,
    rollback_version text,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE node_update_attempts (
    id text PRIMARY KEY,
    node_id text NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    from_version text NOT NULL,
    to_version text NOT NULL,
    state text NOT NULL,
    error_code text,
    rollback_evidence jsonb,
    started_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz
);

CREATE INDEX node_update_attempts_tenant_idx
    ON node_update_attempts (workspace_id, environment_id, started_at DESC);
