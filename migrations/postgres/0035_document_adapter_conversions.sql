-- Reproducible, tenant-scoped hosted adapter conversion records. Source JSON is
-- never retained; only its digest and the encrypted deterministic conversion.
CREATE TABLE document_conversions (
    id text PRIMARY KEY CHECK (char_length(id) BETWEEN 8 AND 80),
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL,
    adapter_id text NOT NULL CHECK (adapter_id = 'pdfme'),
    adapter_version text NOT NULL CHECK (adapter_version = '1.0.0'),
    adapter_api_version text NOT NULL CHECK (adapter_api_version = 'piqae.adapter/v1'),
    source_format text NOT NULL CHECK (source_format = 'pdfme.template'),
    source_sha256 text NOT NULL CHECK (source_sha256 ~ '^[0-9a-f]{64}$'),
    strict boolean NOT NULL,
    fidelity text NOT NULL CHECK (fidelity IN ('exact', 'lossy')),
    renderer_version text NOT NULL CHECK (char_length(renderer_version) BETWEEN 1 AND 80),
    result_ciphertext bytea NOT NULL CHECK (octet_length(result_ciphertext) BETWEEN 29 AND 1048704),
    result_sha256 text NOT NULL CHECK (result_sha256 ~ '^[0-9a-f]{64}$'),
    idempotency_key text NOT NULL CHECK (char_length(idempotency_key) BETWEEN 8 AND 200),
    request_sha256 text NOT NULL CHECK (request_sha256 ~ '^[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (workspace_id, environment_id)
        REFERENCES environments(workspace_id, id) ON DELETE CASCADE,
    UNIQUE (workspace_id, environment_id, id),
    UNIQUE (workspace_id, environment_id, idempotency_key)
);

CREATE INDEX document_conversions_tenant_created_idx
    ON document_conversions (workspace_id, environment_id, created_at DESC, id);

CREATE OR REPLACE VIEW document_encryption_key_references AS
    SELECT document_ciphertext_key_id(draft_ciphertext) AS key_id,
           'template_draft'::text AS resource_type, workspace_id, environment_id, id AS resource_id
      FROM document_templates
    UNION ALL
    SELECT document_ciphertext_key_id(spec_ciphertext),
           'template_revision', workspace_id, environment_id, id
      FROM document_template_revisions
    UNION ALL
    SELECT document_ciphertext_key_id(input_ciphertext),
           'render_input', workspace_id, environment_id, id
      FROM document_renders
    UNION ALL
    SELECT document_ciphertext_key_id(artifact_object_key_ciphertext),
           'render_artifact_reference', workspace_id, environment_id, id
      FROM document_renders WHERE artifact_object_key_ciphertext IS NOT NULL
    UNION ALL
    SELECT document_ciphertext_key_id(result_ciphertext),
           'adapter_conversion_result', workspace_id, environment_id, id
      FROM document_conversions;
