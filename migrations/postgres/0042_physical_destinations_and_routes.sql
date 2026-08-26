-- Model a real printer separately from the operating-system queues that can
-- reach it. All cloud-visible topology remains tenant-scoped: identical local
-- evidence may be projected to several tenants, but no tenant can address or
-- join another tenant's destination, route, job, connector, or node.

ALTER TABLE agents
    ADD CONSTRAINT agents_tenant_id_unique
    UNIQUE (workspace_id, environment_id, id);

ALTER TABLE node_connectors
    ADD CONSTRAINT node_connectors_tenant_id_unique
    UNIQUE (workspace_id, environment_id, id);

ALTER TABLE jobs
    ADD CONSTRAINT jobs_tenant_id_unique
    UNIQUE (workspace_id, environment_id, id);

-- Printer identifiers are tenant-local after migration 41. Preserve ordering
-- independently in every tenant projection of the same local printer ID.
ALTER TABLE jobs
    DROP CONSTRAINT jobs_printer_id_per_printer_sequence_key,
    ADD CONSTRAINT jobs_printer_tenant_sequence_unique
        UNIQUE (workspace_id, environment_id, printer_id, per_printer_sequence);

CREATE TABLE scheduling_authorities (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('hosted_control_plane', 'self_hosted_control_plane', 'site_coordinator')),
    authority_key text NOT NULL,
    display_name text NOT NULL,
    active boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, authority_key),
    FOREIGN KEY (environment_id, workspace_id)
        REFERENCES environments(id, workspace_id) ON DELETE CASCADE
);

CREATE TABLE physical_destinations (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    name text NOT NULL,
    identity_confidence text NOT NULL DEFAULT 'unknown'
        CHECK (identity_confidence IN ('unknown', 'possible', 'high', 'verified', 'conflict')),
    state text NOT NULL DEFAULT 'unknown'
        CHECK (state IN ('unknown', 'available', 'unavailable', 'paused', 'attention', 'retired')),
    scheduling_authority_id text,
    identity_revision bigint NOT NULL DEFAULT 0 CHECK (identity_revision >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    retired_at timestamptz,
    PRIMARY KEY (workspace_id, environment_id, id),
    FOREIGN KEY (environment_id, workspace_id)
        REFERENCES environments(id, workspace_id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, scheduling_authority_id)
        REFERENCES scheduling_authorities(workspace_id, environment_id, id)
);

CREATE INDEX physical_destinations_tenant_state_idx
    ON physical_destinations (workspace_id, environment_id, state, updated_at DESC, id);

CREATE TABLE printer_routes (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    destination_id text NOT NULL,
    printer_id text NOT NULL,
    agent_id text NOT NULL,
    native_queue_id text NOT NULL,
    local_route_key text,
    state text NOT NULL DEFAULT 'unknown'
        CHECK (state IN ('unknown', 'available', 'unavailable', 'paused', 'rejecting', 'stale', 'retired')),
    role text NOT NULL DEFAULT 'standby' CHECK (role IN ('primary', 'standby', 'disabled')),
    priority integer NOT NULL DEFAULT 100 CHECK (priority BETWEEN 0 AND 1000000),
    enabled boolean NOT NULL DEFAULT true,
    capability_revision bigint NOT NULL DEFAULT 0 CHECK (capability_revision >= 0),
    profile_revision bigint NOT NULL DEFAULT 0 CHECK (profile_revision >= 0),
    last_seen_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    retired_at timestamptz,
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, printer_id, agent_id),
    UNIQUE (workspace_id, environment_id, agent_id, local_route_key),
    UNIQUE (workspace_id, environment_id, destination_id, id),
    FOREIGN KEY (workspace_id, environment_id, destination_id)
        REFERENCES physical_destinations(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, printer_id)
        REFERENCES printers(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, agent_id)
        REFERENCES agents(workspace_id, environment_id, id) ON DELETE CASCADE
);

CREATE INDEX printer_routes_destination_health_idx
    ON printer_routes (workspace_id, environment_id, destination_id, enabled, state, priority, id)
    WHERE retired_at IS NULL;

CREATE UNIQUE INDEX printer_routes_one_primary_idx
    ON printer_routes (workspace_id, environment_id, destination_id)
    WHERE role = 'primary' AND enabled AND retired_at IS NULL;

CREATE TABLE destination_identity_evidence (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    destination_id text NOT NULL,
    route_id text NOT NULL,
    kind text NOT NULL CHECK (kind IN (
        'ipp_uuid', 'device_serial', 'usb_serial', 'usb_vid_pid',
        'certificate_key', 'network_mac', 'network_endpoint', 'native_queue',
        'manufacturer_model', 'driver_fingerprint', 'capability_fingerprint',
        'operator_confirmation'
    )),
    value_digest text NOT NULL
        CHECK (value_digest ~ '^hmac-sha256:[0-9a-f]{64}$'),
    strength text NOT NULL CHECK (strength IN ('weak', 'medium', 'strong')),
    conflicts boolean NOT NULL DEFAULT false,
    observed_at timestamptz NOT NULL,
    expires_at timestamptz,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(metadata) = 'object'),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, route_id, kind, value_digest),
    FOREIGN KEY (workspace_id, environment_id, destination_id)
        REFERENCES physical_destinations(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, id) ON DELETE CASCADE,
    CHECK ((metadata - 'source' - 'schema_version' - 'normalization' - 'key_version') = '{}'::jsonb),
    CHECK (NOT (metadata ? 'source') OR metadata->>'source' IN ('node', 'operator', 'migration')),
    CHECK (NOT (metadata ? 'normalization') OR char_length(metadata->>'normalization') BETWEEN 1 AND 64),
    CHECK (NOT (metadata ? 'key_version') OR metadata->>'key_version' ~ '^v[0-9]{1,6}$')
);

CREATE INDEX destination_identity_evidence_match_idx
    ON destination_identity_evidence
       (workspace_id, environment_id, kind, value_digest, conflicts, observed_at DESC);

CREATE TABLE destination_identity_decisions (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('merge', 'split', 'confirm', 'reject_match', 'reverse')),
    destination_id text NOT NULL,
    related_destination_ids jsonb NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(related_destination_ids) = 'array' AND jsonb_array_length(related_destination_ids) <= 64),
    evidence_ids jsonb NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(evidence_ids) = 'array' AND jsonb_array_length(evidence_ids) <= 256),
    effect_snapshot jsonb NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(effect_snapshot) = 'object'),
    actor_kind text NOT NULL CHECK (actor_kind IN ('system', 'operator', 'migration')),
    actor_id text,
    reason text NOT NULL CHECK (char_length(reason) BETWEEN 1 AND 2048),
    reverses_decision_id text,
    request_id text,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    FOREIGN KEY (workspace_id, environment_id, destination_id)
        REFERENCES physical_destinations(workspace_id, environment_id, id),
    FOREIGN KEY (workspace_id, environment_id, reverses_decision_id)
        REFERENCES destination_identity_decisions(workspace_id, environment_id, id),
    CHECK ((kind = 'reverse') = (reverses_decision_id IS NOT NULL))
);

CREATE INDEX destination_identity_decisions_audit_idx
    ON destination_identity_decisions
       (workspace_id, environment_id, destination_id, created_at, id);

CREATE UNIQUE INDEX destination_identity_decisions_one_reversal_idx
    ON destination_identity_decisions
       (workspace_id, environment_id, reverses_decision_id)
    WHERE reverses_decision_id IS NOT NULL;

CREATE TABLE destination_identity_decision_routes (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    decision_id text NOT NULL,
    route_id text NOT NULL,
    PRIMARY KEY (workspace_id, environment_id, decision_id, route_id),
    FOREIGN KEY (workspace_id, environment_id, decision_id)
        REFERENCES destination_identity_decisions(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, id)
);

CREATE TABLE route_observations (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    route_id text NOT NULL,
    sequence bigint NOT NULL CHECK (sequence > 0),
    printer_state text NOT NULL CHECK (printer_state IN ('unknown', 'idle', 'processing', 'stopped', 'unavailable')),
    accepting_jobs boolean,
    state_reasons text[] NOT NULL DEFAULT '{}',
    total_jobs integer NOT NULL DEFAULT 0 CHECK (total_jobs >= 0),
    connector_jobs integer NOT NULL DEFAULT 0 CHECK (connector_jobs >= 0),
    other_piqae_or_external_jobs integer NOT NULL DEFAULT 0
        CHECK (other_piqae_or_external_jobs >= 0),
    unknown_jobs integer NOT NULL DEFAULT 0 CHECK (unknown_jobs >= 0),
    active_jobs integer NOT NULL DEFAULT 0 CHECK (active_jobs >= 0),
    held_jobs integer NOT NULL DEFAULT 0 CHECK (held_jobs >= 0),
    estimated_busy_seconds integer CHECK (estimated_busy_seconds >= 0),
    privacy_level text NOT NULL DEFAULT 'counts_only' CHECK (privacy_level IN ('counts_only', 'piqae_details')),
    stock_state jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(stock_state) = 'object'),
    observed_at timestamptz NOT NULL,
    fresh_until timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, route_id, sequence),
    FOREIGN KEY (workspace_id, environment_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, id) ON DELETE CASCADE,
    CHECK (fresh_until >= observed_at),
    CHECK (cardinality(state_reasons) <= 64),
    CHECK (total_jobs = connector_jobs + other_piqae_or_external_jobs),
    CHECK (unknown_jobs <= other_piqae_or_external_jobs),
    CHECK (active_jobs <= total_jobs AND held_jobs <= total_jobs)
);

CREATE INDEX route_observations_latest_idx
    ON route_observations (workspace_id, environment_id, route_id, observed_at DESC, id DESC);

CREATE TABLE projection_acknowledgements (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    agent_id text NOT NULL,
    route_id text NOT NULL,
    inventory_revision bigint NOT NULL CHECK (inventory_revision >= 0),
    capability_revision bigint NOT NULL CHECK (capability_revision >= 0),
    status text NOT NULL CHECK (status IN ('pending', 'acknowledged', 'rejected')),
    error_code text,
    observed_at timestamptz NOT NULL,
    acknowledged_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, agent_id, route_id),
    FOREIGN KEY (workspace_id, environment_id, agent_id)
        REFERENCES agents(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, id) ON DELETE CASCADE,
    CHECK ((status = 'acknowledged') = (acknowledged_at IS NOT NULL)),
    CHECK (error_code IS NULL OR char_length(error_code) BETWEEN 1 AND 128)
);

CREATE TABLE delivery_attempts (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    job_id text NOT NULL,
    destination_id text NOT NULL,
    route_id text NOT NULL,
    generation bigint NOT NULL CHECK (generation > 0),
    fencing_token_hash text NOT NULL CHECK (char_length(fencing_token_hash) BETWEEN 32 AND 128),
    state text NOT NULL CHECK (state IN (
        'route_leased', 'accepted_by_node', 'queued_local', 'handing_to_spooler',
        'accepted_by_spooler', 'printing_reported', 'completed_reported',
        'cancelled', 'failed', 'delivery_uncertain', 'superseded'
    )),
    lease_until timestamptz NOT NULL,
    accepted_at timestamptz,
    handoff_started_at timestamptz,
    spooler_accepted_at timestamptz,
    native_spool_id_ciphertext bytea,
    final_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, job_id, generation),
    FOREIGN KEY (workspace_id, environment_id, job_id)
        REFERENCES jobs(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, destination_id)
        REFERENCES physical_destinations(workspace_id, environment_id, id),
    FOREIGN KEY (workspace_id, environment_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, id),
    CHECK ((final_at IS NULL) = (state NOT IN ('completed_reported', 'cancelled', 'failed', 'delivery_uncertain', 'superseded')))
);

CREATE UNIQUE INDEX delivery_attempts_one_active_job_idx
    ON delivery_attempts (workspace_id, environment_id, job_id)
    WHERE final_at IS NULL;

CREATE INDEX delivery_attempts_route_state_idx
    ON delivery_attempts (workspace_id, environment_id, route_id, state, lease_until)
    WHERE final_at IS NULL;

CREATE TABLE route_reservations (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    route_id text NOT NULL,
    destination_id text NOT NULL,
    job_id text NOT NULL,
    attempt_id text NOT NULL,
    generation bigint NOT NULL CHECK (generation > 0),
    fencing_token_hash text NOT NULL CHECK (char_length(fencing_token_hash) BETWEEN 32 AND 128),
    state text NOT NULL CHECK (state IN ('active', 'released', 'expired', 'cancelled', 'superseded')),
    lease_until timestamptz NOT NULL,
    released_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, attempt_id),
    FOREIGN KEY (workspace_id, environment_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, id),
    FOREIGN KEY (workspace_id, environment_id, destination_id)
        REFERENCES physical_destinations(workspace_id, environment_id, id),
    FOREIGN KEY (workspace_id, environment_id, job_id)
        REFERENCES jobs(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, attempt_id)
        REFERENCES delivery_attempts(workspace_id, environment_id, id) ON DELETE CASCADE,
    CHECK ((state = 'active') = (released_at IS NULL))
);

CREATE UNIQUE INDEX route_reservations_one_active_route_idx
    ON route_reservations (workspace_id, environment_id, route_id)
    WHERE state = 'active';

CREATE UNIQUE INDEX route_reservations_one_active_destination_idx
    ON route_reservations (workspace_id, environment_id, destination_id)
    WHERE state = 'active';

CREATE UNIQUE INDEX route_reservations_one_active_job_idx
    ON route_reservations (workspace_id, environment_id, job_id)
    WHERE state = 'active';

-- Ambiguous handoffs remain blocked until a durable operator decision exists.
-- The resolution is append-only and idempotent by request ID; reprinting is a
-- separate, explicit job and is never performed by this migration/repository.
CREATE TABLE delivery_uncertainty_resolutions (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    id text NOT NULL,
    job_id text NOT NULL,
    attempt_id text NOT NULL,
    destination_id text NOT NULL,
    resolution text NOT NULL CHECK (resolution IN (
        'confirmed_delivered', 'reprint_authorized', 'accept_missing', 'cancelled'
    )),
    note text CHECK (note IS NULL OR char_length(note) BETWEEN 1 AND 2048),
    actor_id text NOT NULL CHECK (char_length(actor_id) BETWEEN 1 AND 256),
    request_id text NOT NULL CHECK (char_length(request_id) BETWEEN 1 AND 256),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, attempt_id),
    UNIQUE (workspace_id, environment_id, request_id),
    FOREIGN KEY (workspace_id, environment_id, job_id)
        REFERENCES jobs(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, attempt_id)
        REFERENCES delivery_attempts(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, destination_id)
        REFERENCES physical_destinations(workspace_id, environment_id, id)
);

CREATE INDEX delivery_uncertainty_resolutions_destination_idx
    ON delivery_uncertainty_resolutions
       (workspace_id, environment_id, destination_id, created_at DESC, id);

CREATE TABLE site_coordinator_memberships (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    authority_id text NOT NULL,
    agent_id text NOT NULL,
    site_id text NOT NULL,
    state text NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'draining', 'revoked')),
    joined_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz,
    revoked_at timestamptz,
    PRIMARY KEY (workspace_id, environment_id, authority_id, agent_id),
    FOREIGN KEY (workspace_id, environment_id, authority_id)
        REFERENCES scheduling_authorities(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, agent_id)
        REFERENCES agents(workspace_id, environment_id, id) ON DELETE CASCADE,
    CHECK ((state = 'revoked') = (revoked_at IS NOT NULL))
);

CREATE INDEX site_coordinator_memberships_site_idx
    ON site_coordinator_memberships
       (workspace_id, environment_id, site_id, state, last_seen_at DESC);

-- Backfill a conservative one-route destination for every existing printer.
-- No cross-route or cross-tenant grouping is inferred from names or addresses.
INSERT INTO scheduling_authorities (
    workspace_id, environment_id, id, kind, authority_key, display_name
)
SELECT DISTINCT workspace_id, environment_id,
       'sa_' || environment_id,
       'hosted_control_plane',
       'environment:' || environment_id,
       'Existing control plane'
FROM printers
ON CONFLICT (workspace_id, environment_id, authority_key) DO NOTHING;

INSERT INTO physical_destinations (
    workspace_id, environment_id, id, name, identity_confidence,
    state, scheduling_authority_id
)
SELECT workspace_id, environment_id,
       'pdst_' || md5(workspace_id || ':' || environment_id || ':' || id),
       name,
       'unknown',
       CASE
           WHEN state = 'online' THEN 'available'
           WHEN state = 'offline' THEN 'unavailable'
           ELSE 'unknown'
       END,
       'sa_' || environment_id
FROM printers
ON CONFLICT (workspace_id, environment_id, id) DO NOTHING;

INSERT INTO printer_routes (
    workspace_id, environment_id, id, destination_id, printer_id,
    agent_id, native_queue_id, state, role, priority, enabled,
    local_route_key, capability_revision, last_seen_at
)
SELECT workspace_id, environment_id,
       'rte_' || md5(workspace_id || ':' || environment_id || ':' || id),
       'pdst_' || md5(workspace_id || ':' || environment_id || ':' || id),
       id,
       agent_id,
       native_id,
       CASE
           WHEN state = 'online' THEN 'available'
           WHEN state = 'offline' THEN 'unavailable'
           ELSE 'unknown'
       END,
       'primary', 0, removed_at IS NULL,
       NULL,
       capabilities_revision, last_seen_at
FROM printers
ON CONFLICT (workspace_id, environment_id, printer_id, agent_id) DO NOTHING;

-- Existing logical targets become route-aware. The compatibility printer and
-- agent columns remain temporarily readable. destination_id is authoritative;
-- route_id records the preferred/profile
-- source route; the scheduler may select another compatible healthy route.
ALTER TABLE target_bindings
    ADD COLUMN destination_id text,
    ADD COLUMN route_id text;

UPDATE target_bindings binding
SET destination_id = route.destination_id,
    route_id = route.id
FROM printer_routes route
WHERE route.workspace_id = binding.workspace_id
  AND route.environment_id = binding.environment_id
  AND route.printer_id = binding.printer_id
  AND route.agent_id = binding.agent_id;

ALTER TABLE target_bindings
    ADD CONSTRAINT target_bindings_destination_tenant_fkey
        FOREIGN KEY (workspace_id, environment_id, destination_id)
        REFERENCES physical_destinations(workspace_id, environment_id, id),
    ADD CONSTRAINT target_bindings_route_tenant_fkey
        FOREIGN KEY (workspace_id, environment_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, id),
    ADD CONSTRAINT target_bindings_destination_route_fkey
        FOREIGN KEY (workspace_id, environment_id, destination_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, destination_id, id)
        DEFERRABLE INITIALLY DEFERRED;

CREATE INDEX target_bindings_destination_route_idx
    ON target_bindings
       (workspace_id, environment_id, destination_id, role, route_id)
    WHERE enabled;

-- Compatibility jobs are backfilled to the route that represented their
-- original printer/agent pair. New fenced scheduling writes these fields and
-- delivery_attempts; old supported callers can continue during the rollout.
ALTER TABLE jobs
    ADD COLUMN destination_id text,
    ADD COLUMN route_id text;

UPDATE jobs job
SET destination_id = route.destination_id,
    route_id = route.id
FROM printer_routes route
WHERE route.workspace_id = job.workspace_id
  AND route.environment_id = job.environment_id
  AND route.printer_id = job.printer_id
  AND route.agent_id = job.agent_id;

ALTER TABLE jobs
    ADD CONSTRAINT jobs_destination_tenant_fkey
        FOREIGN KEY (workspace_id, environment_id, destination_id)
        REFERENCES physical_destinations(workspace_id, environment_id, id),
    ADD CONSTRAINT jobs_route_tenant_fkey
        FOREIGN KEY (workspace_id, environment_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, id),
    ADD CONSTRAINT jobs_destination_route_fkey
        FOREIGN KEY (workspace_id, environment_id, destination_id, route_id)
        REFERENCES printer_routes(workspace_id, environment_id, destination_id, id)
        DEFERRABLE INITIALLY DEFERRED;

CREATE INDEX jobs_destination_route_idx
    ON jobs (workspace_id, environment_id, destination_id, route_id, created_at DESC);
