CREATE TABLE stocks (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name text NOT NULL,
    sku text,
    description text,
    attributes jsonb NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(attributes) = 'object'),
    archived boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, environment_id, name)
);

CREATE INDEX stocks_tenant_idx
    ON stocks (workspace_id, environment_id, created_at, id);

CREATE TABLE targets (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name text NOT NULL,
    description text,
    stock_id text REFERENCES stocks(id),
    enabled boolean NOT NULL DEFAULT true,
    routing_policy text NOT NULL DEFAULT 'primary_then_standby'
        CHECK (routing_policy IN ('primary_then_standby')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, environment_id, name)
);

CREATE INDEX targets_tenant_idx
    ON targets (workspace_id, environment_id, created_at, id);

CREATE TABLE target_bindings (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    target_id text NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    printer_id text NOT NULL REFERENCES printers(id),
    agent_id text NOT NULL REFERENCES agents(id),
    profile_id text NOT NULL,
    profile_revision bigint NOT NULL CHECK (profile_revision > 0),
    role text NOT NULL CHECK (role IN ('primary', 'standby')),
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (target_id, printer_id, profile_id, profile_revision),
    UNIQUE (target_id, role)
);

CREATE INDEX target_bindings_tenant_idx
    ON target_bindings (workspace_id, environment_id, target_id, role);
