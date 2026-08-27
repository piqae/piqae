-- Embedded and mobile runtimes report bounded lifecycle availability separately
-- from printer telemetry. A wake hint is only advisory: it cannot own a job,
-- reservation, lease, fencing token, document, or cross-tenant identity.

CREATE TABLE node_runtime_observations (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    agent_id text NOT NULL,
    sequence bigint NOT NULL CHECK (sequence > 0),
    host_mode text NOT NULL
        CHECK (host_mode IN ('machine_service', 'user_agent', 'embedded_application', 'attached_client')),
    availability_class text NOT NULL CHECK (availability_class IN (
        'continuous_while_awake', 'foreground_only', 'background_opportunistic',
        'managed_kiosk', 'wake_relay_capable'
    )),
    lifecycle_state text NOT NULL CHECK (lifecycle_state IN (
        'available', 'foreground', 'background', 'suspending', 'suspended',
        'waking', 'unavailable'
    )),
    accepts_cloud_jobs boolean NOT NULL,
    execution_budget_ms bigint CHECK (execution_budget_ms >= 0),
    wake_mechanisms text[] NOT NULL DEFAULT '{}',
    observed_at timestamptz NOT NULL,
    fresh_until timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, agent_id, sequence),
    FOREIGN KEY (workspace_id, environment_id, agent_id)
        REFERENCES agents(workspace_id, environment_id, id) ON DELETE CASCADE,
    CHECK (fresh_until >= observed_at),
    CHECK (fresh_until <= observed_at + interval '10 minutes'),
    CHECK (cardinality(wake_mechanisms) <= 8),
    CHECK (wake_mechanisms <@ ARRAY[
        'local_broker', 'apns_background', 'bluetooth_accessory',
        'external_accessory', 'wake_on_lan', 'manual'
    ]::text[]),
    CHECK (
        availability_class <> 'background_opportunistic'
        OR NOT accepts_cloud_jobs
        OR execution_budget_ms >= 30000
    ),
    CHECK (
        NOT accepts_cloud_jobs
        OR lifecycle_state IN ('available', 'foreground', 'background')
    ),
    CHECK (
        availability_class <> 'foreground_only'
        OR NOT accepts_cloud_jobs
        OR lifecycle_state = 'foreground'
    ),
    CHECK (host_mode <> 'attached_client' OR NOT accepts_cloud_jobs)
);

CREATE INDEX node_runtime_observations_latest_idx
    ON node_runtime_observations
       (workspace_id, environment_id, agent_id, sequence DESC);

CREATE TABLE node_wake_hints (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    agent_id text NOT NULL,
    idempotency_key text NOT NULL CHECK (char_length(idempotency_key) BETWEEN 8 AND 255),
    reason text NOT NULL CHECK (reason IN (
        'job_available', 'operator_request', 'inventory_refresh', 'diagnostics'
    )),
    delivery_channel text NOT NULL DEFAULT 'connected_session'
        CHECK (delivery_channel IN ('connected_session', 'external_push', 'local_relay', 'manual')),
    status text NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'observed', 'expired', 'cancelled')),
    requested_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    observed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, agent_id, idempotency_key),
    FOREIGN KEY (workspace_id, environment_id, agent_id)
        REFERENCES agents(workspace_id, environment_id, id) ON DELETE CASCADE,
    CHECK (expires_at > requested_at),
    CHECK (expires_at <= requested_at + interval '15 minutes'),
    CHECK ((status = 'observed') = (observed_at IS NOT NULL)),
    CHECK (observed_at IS NULL OR observed_at >= requested_at)
);

CREATE INDEX node_wake_hints_pending_idx
    ON node_wake_hints
       (workspace_id, environment_id, agent_id, expires_at, requested_at, id)
    WHERE status = 'pending';
