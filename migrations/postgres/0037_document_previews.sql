-- Durable approval gates for immutable document artifacts. No print job exists
-- until approval. Tenant columns are repeated on every relationship so an
-- opaque identifier can never cross a workspace/environment boundary.
CREATE TABLE document_previews (
    id text NOT NULL,
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    render_id text NOT NULL,
    state text NOT NULL DEFAULT 'awaiting_approval'
        CHECK (state IN ('awaiting_approval','approving','approved','cancelled','expired')),
    idempotency_key text NOT NULL CHECK (char_length(idempotency_key) BETWEEN 8 AND 200),
    request_sha256 text NOT NULL CHECK (request_sha256 ~ '^[0-9a-f]{64}$'),
    approval_request_sha256 text CHECK (approval_request_sha256 IS NULL OR approval_request_sha256 ~ '^[0-9a-f]{64}$'),
    approval_idempotency_key text CHECK (approval_idempotency_key IS NULL OR char_length(approval_idempotency_key) BETWEEN 8 AND 200),
    job_id text,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, idempotency_key),
    FOREIGN KEY (workspace_id, environment_id, render_id)
        REFERENCES document_renders (workspace_id, environment_id, id) ON DELETE RESTRICT,
    FOREIGN KEY (workspace_id, environment_id, job_id)
        REFERENCES jobs (workspace_id, environment_id, id) ON DELETE RESTRICT,
    CHECK (expires_at > created_at),
    CHECK ((state = 'approved') = (job_id IS NOT NULL)),
    CHECK (state <> 'approving' OR (approval_request_sha256 IS NOT NULL AND approval_idempotency_key IS NOT NULL))
);

CREATE INDEX document_previews_expiry_idx
    ON document_previews (expires_at, id)
    WHERE state IN ('awaiting_approval','approving');
CREATE INDEX document_previews_render_idx
    ON document_previews (workspace_id, environment_id, render_id, expires_at)
    WHERE state IN ('awaiting_approval','approving');
