-- Optional, tenant-isolated document templates and render registrations.
-- Sensitive payloads are application-encrypted before they reach PostgreSQL.

CREATE TABLE document_templates (
    id text PRIMARY KEY CHECK (char_length(id) BETWEEN 8 AND 80),
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL,
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    state text NOT NULL DEFAULT 'draft' CHECK (state IN ('draft', 'published')),
    draft_ciphertext bytea NOT NULL CHECK (octet_length(draft_ciphertext) BETWEEN 29 AND 1048605),
    draft_sha256 text NOT NULL CHECK (draft_sha256 ~ '^[0-9a-f]{64}$'),
    published_revision_id text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (workspace_id, environment_id)
        REFERENCES environments(workspace_id, id) ON DELETE CASCADE,
    UNIQUE (workspace_id, environment_id, id)
);

CREATE INDEX document_templates_tenant_created_idx
    ON document_templates (workspace_id, environment_id, created_at DESC, id);

CREATE TABLE document_template_revisions (
    id text PRIMARY KEY CHECK (char_length(id) BETWEEN 8 AND 80),
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL,
    template_id text NOT NULL,
    revision integer NOT NULL CHECK (revision > 0),
    spec_ciphertext bytea NOT NULL CHECK (octet_length(spec_ciphertext) BETWEEN 29 AND 1048605),
    spec_sha256 text NOT NULL CHECK (spec_sha256 ~ '^[0-9a-f]{64}$'),
    renderer_profile text NOT NULL CHECK (renderer_profile = 'printpacket/v1'),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (workspace_id, environment_id, template_id)
        REFERENCES document_templates(workspace_id, environment_id, id) ON DELETE CASCADE,
    UNIQUE (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, template_id, revision)
);

ALTER TABLE document_templates
    ADD CONSTRAINT document_templates_published_revision_fk
    FOREIGN KEY (workspace_id, environment_id, published_revision_id)
    REFERENCES document_template_revisions(workspace_id, environment_id, id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE document_renders (
    id text PRIMARY KEY CHECK (char_length(id) BETWEEN 8 AND 80),
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL,
    template_revision_id text NOT NULL,
    input_ciphertext bytea NOT NULL CHECK (octet_length(input_ciphertext) BETWEEN 29 AND 1048605),
    input_sha256 text NOT NULL CHECK (input_sha256 ~ '^[0-9a-f]{64}$'),
    state text NOT NULL DEFAULT 'registered'
        CHECK (state IN ('registered', 'rendering', 'completed', 'failed')),
    artifact_object_key_ciphertext bytea,
    artifact_sha256 text CHECK (artifact_sha256 IS NULL OR artifact_sha256 ~ '^[0-9a-f]{64}$'),
    artifact_byte_length bigint CHECK (artifact_byte_length IS NULL OR artifact_byte_length BETWEEN 1 AND 52428800),
    artifact_media_type text CHECK (artifact_media_type IS NULL OR artifact_media_type = 'application/pdf'),
    failure_code text CHECK (failure_code IS NULL OR char_length(failure_code) BETWEEN 1 AND 80),
    idempotency_key text NOT NULL CHECK (char_length(idempotency_key) BETWEEN 8 AND 200),
    request_sha256 text NOT NULL CHECK (request_sha256 ~ '^[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (workspace_id, environment_id)
        REFERENCES environments(workspace_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, template_revision_id)
        REFERENCES document_template_revisions(workspace_id, environment_id, id) ON DELETE CASCADE,
    CHECK (artifact_object_key_ciphertext IS NULL OR octet_length(artifact_object_key_ciphertext) BETWEEN 29 AND 1045),
    UNIQUE (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, idempotency_key)
);

CREATE INDEX document_renders_tenant_created_idx
    ON document_renders (workspace_id, environment_id, created_at DESC, id);
CREATE INDEX document_renders_pending_idx
    ON document_renders (created_at, id)
    WHERE state = 'registered';
