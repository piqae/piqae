CREATE TABLE workspaces (
    id text PRIMARY KEY,
    name text NOT NULL,
    workos_organization_id text UNIQUE,
    settings jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE users (
    id text PRIMARY KEY,
    workos_user_id text UNIQUE,
    email text NOT NULL,
    display_name text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE workspace_members (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role text NOT NULL CHECK (role IN ('owner', 'admin', 'developer', 'operator', 'viewer', 'billing')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);

CREATE TABLE environments (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('test', 'live')),
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, kind)
);

CREATE TABLE api_keys (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name text NOT NULL,
    lookup_prefix text NOT NULL UNIQUE,
    secret_hash text NOT NULL,
    scopes text[] NOT NULL,
    expires_at timestamptz,
    last_used_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE enrolment_tokens (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    secret_hash text NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE agents (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name text NOT NULL,
    installation_id text NOT NULL,
    public_key bytea,
    os text NOT NULL,
    architecture text NOT NULL,
    version text NOT NULL,
    protocol_version integer NOT NULL,
    state text NOT NULL DEFAULT 'offline',
    last_seen_at timestamptz,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, environment_id, installation_id)
);

CREATE TABLE agent_event_receipts (
    agent_id text NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    event_id text NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_id, event_id)
);

CREATE TABLE printers (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    agent_id text NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    native_id text NOT NULL,
    name text NOT NULL,
    state text NOT NULL DEFAULT 'unknown',
    state_reasons jsonb NOT NULL DEFAULT '[]'::jsonb,
    capabilities jsonb NOT NULL DEFAULT '{}'::jsonb,
    capabilities_revision bigint NOT NULL DEFAULT 0,
    is_default boolean NOT NULL DEFAULT false,
    last_seen_at timestamptz,
    removed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (agent_id, native_id)
);

CREATE TABLE contents (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    object_key text NOT NULL UNIQUE,
    sha256 text NOT NULL,
    byte_length bigint NOT NULL CHECK (byte_length >= 0),
    media_type text NOT NULL,
    retained_until timestamptz NOT NULL,
    deleted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE jobs (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    printer_id text NOT NULL REFERENCES printers(id),
    agent_id text NOT NULL REFERENCES agents(id),
    content_id text REFERENCES contents(id),
    payload jsonb NOT NULL,
    state text NOT NULL,
    state_sequence bigint NOT NULL DEFAULT 0 CHECK (state_sequence >= 0),
    per_printer_sequence bigint NOT NULL,
    lease_owner text,
    lease_until timestamptz,
    expires_at timestamptz NOT NULL,
    final_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (printer_id, per_printer_sequence)
);

CREATE INDEX jobs_route_idx
    ON jobs (agent_id, state, created_at)
    WHERE final_at IS NULL;
CREATE INDEX jobs_tenant_idx ON jobs (workspace_id, environment_id, created_at DESC);
CREATE INDEX jobs_lease_idx ON jobs (lease_until) WHERE lease_owner IS NOT NULL;

CREATE TABLE job_events (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    job_id text NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    sequence bigint NOT NULL CHECK (sequence > 0),
    state text NOT NULL,
    payload jsonb NOT NULL,
    occurred_at timestamptz NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (job_id, sequence)
);

CREATE INDEX job_events_tenant_cursor_idx
    ON job_events (workspace_id, environment_id, received_at, id);

CREATE TABLE idempotency_requests (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    operation text NOT NULL,
    key text NOT NULL,
    request_hash text NOT NULL,
    resource_id text,
    response_status integer,
    response_body jsonb,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, operation, key)
);

CREATE TABLE compatibility_ids (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    resource_type text NOT NULL,
    resource_id text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, environment_id, resource_type, resource_id)
);

CREATE TABLE routing_outbox (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    aggregate_type text NOT NULL,
    aggregate_id text NOT NULL,
    event_type text NOT NULL,
    payload jsonb NOT NULL,
    available_at timestamptz NOT NULL DEFAULT now(),
    attempts integer NOT NULL DEFAULT 0,
    claimed_until timestamptz,
    processed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX routing_outbox_ready_idx
    ON routing_outbox (available_at, created_at)
    WHERE processed_at IS NULL;

CREATE TABLE webhook_endpoints (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    url text NOT NULL,
    description text,
    secret_ciphertext bytea NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    subscribed_events text[] NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE webhook_events (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    event_type text NOT NULL,
    payload jsonb NOT NULL,
    occurred_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE webhook_deliveries (
    id text PRIMARY KEY,
    endpoint_id text NOT NULL REFERENCES webhook_endpoints(id) ON DELETE CASCADE,
    event_id text NOT NULL REFERENCES webhook_events(id) ON DELETE CASCADE,
    destination_url text NOT NULL,
    attempt integer NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    response_status integer,
    response_excerpt text,
    delivered_at timestamptz,
    dead_lettered_at timestamptz,
    claimed_until timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (endpoint_id, event_id)
);

CREATE INDEX webhook_deliveries_ready_idx
    ON webhook_deliveries (next_attempt_at)
    WHERE delivered_at IS NULL AND dead_lettered_at IS NULL;

CREATE TABLE audit_events (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    environment_id text,
    actor_type text NOT NULL,
    actor_id text,
    action text NOT NULL,
    resource_type text,
    resource_id text,
    safe_metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    request_id text,
    occurred_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX audit_events_tenant_idx
    ON audit_events (workspace_id, occurred_at DESC, id);

CREATE TABLE usage_ledger (
    id text PRIMARY KEY,
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    job_id text,
    kind text NOT NULL,
    units bigint NOT NULL,
    reason text,
    occurred_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX usage_one_acceptance_per_job_idx
    ON usage_ledger (job_id)
    WHERE kind = 'print_job_accepted' AND job_id IS NOT NULL;

CREATE TABLE retention_runs (
    id text PRIMARY KEY,
    started_at timestamptz NOT NULL,
    completed_at timestamptz,
    deleted_objects bigint NOT NULL DEFAULT 0,
    deleted_bytes bigint NOT NULL DEFAULT 0,
    error text
);

