-- Persist the bounded renderer/cache capability last reported by each node.
-- The projection is never authoritative: every node must still validate an
-- offered manifest and fail closed before durable acceptance.
ALTER TABLE agents
    ADD COLUMN printpacket_render_capabilities jsonb NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE agents
    ADD CONSTRAINT agents_printpacket_render_capabilities_object
    CHECK (jsonb_typeof(printpacket_render_capabilities) = 'object');

ALTER TABLE document_renders
    ADD COLUMN page_count integer CHECK (page_count BETWEEN 1 AND 100000);

CREATE TABLE printpacket_resources (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    digest text NOT NULL CHECK (digest ~ '^[a-f0-9]{64}$'),
    media_type text NOT NULL CHECK (media_type = 'image/jpeg'),
    byte_length bigint NOT NULL CHECK (byte_length BETWEEN 1 AND 4194304),
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT now() + interval '30 days',
    cleanup_state text NOT NULL DEFAULT 'active' CHECK (cleanup_state IN ('active','expiring')),
    cleanup_lease_until timestamptz,
    cleanup_lease_token uuid,
    PRIMARY KEY (workspace_id, environment_id, digest)
);

CREATE TABLE printpacket_resource_references (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    render_id text NOT NULL,
    resource_digest text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, render_id, resource_digest),
    FOREIGN KEY (workspace_id, environment_id, resource_digest)
        REFERENCES printpacket_resources(workspace_id, environment_id, digest) ON DELETE RESTRICT,
    FOREIGN KEY (render_id, workspace_id, environment_id)
        REFERENCES document_renders(id, workspace_id, environment_id) ON DELETE CASCADE
);

CREATE INDEX printpacket_resources_expiry_idx
    ON printpacket_resources(expires_at)
    WHERE expires_at IS NOT NULL;
