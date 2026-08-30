-- Expiring, unpublished PrintPacket preview renders share the production render
-- queue and artifact lifecycle. Sensitive specification and input bytes remain
-- application-encrypted; purpose is an irreversible print/approval fence.

ALTER TABLE document_renders
    ALTER COLUMN template_revision_id DROP NOT NULL,
    ALTER COLUMN input_ciphertext DROP NOT NULL,
    ALTER COLUMN input_sha256 DROP NOT NULL,
    ADD COLUMN purpose text NOT NULL DEFAULT 'printable'
        CHECK (purpose IN ('printable', 'preview')),
    ADD COLUMN spec_ciphertext bytea,
    ADD COLUMN spec_sha256 text;

ALTER TABLE document_renders
    ADD CONSTRAINT document_renders_spec_ciphertext_check
        CHECK (spec_ciphertext IS NULL OR octet_length(spec_ciphertext) BETWEEN 29 AND 1048704),
    ADD CONSTRAINT document_renders_spec_sha256_check
        CHECK (spec_sha256 IS NULL OR spec_sha256 ~ '^[0-9a-f]{64}$'),
    ADD CONSTRAINT document_renders_source_shape_check CHECK (
        (purpose = 'printable' AND template_revision_id IS NOT NULL)
        OR (purpose = 'preview' AND template_revision_id IS NULL)
    ),
    ADD CONSTRAINT document_renders_sensitive_payload_shape_check CHECK (
        (state = 'expired'
            AND input_ciphertext IS NULL
            AND input_sha256 IS NULL
            AND spec_ciphertext IS NULL
            AND spec_sha256 IS NULL)
        OR
        (state <> 'expired'
            AND input_ciphertext IS NOT NULL
            AND input_sha256 IS NOT NULL
            AND (
                (purpose = 'printable' AND spec_ciphertext IS NULL AND spec_sha256 IS NULL)
                OR
                (purpose = 'preview'
                    AND spec_ciphertext IS NOT NULL
                    AND spec_sha256 IS NOT NULL
                    AND expires_at >= created_at + interval '60 seconds'
                    AND expires_at <= created_at + interval '1800 seconds')
            ))
    );

CREATE INDEX document_renders_preview_expiry_idx
    ON document_renders (workspace_id, environment_id, expires_at, id)
    WHERE purpose = 'preview' AND state <> 'expired';

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
      FROM document_renders WHERE input_ciphertext IS NOT NULL AND purpose = 'printable'
    UNION ALL
    SELECT document_ciphertext_key_id(input_ciphertext),
           'render_preview_input', workspace_id, environment_id, id
      FROM document_renders WHERE input_ciphertext IS NOT NULL AND purpose = 'preview'
    UNION ALL
    SELECT document_ciphertext_key_id(spec_ciphertext),
           'render_preview_specification', workspace_id, environment_id, id
      FROM document_renders WHERE spec_ciphertext IS NOT NULL
    UNION ALL
    SELECT document_ciphertext_key_id(artifact_object_key_ciphertext),
           'render_artifact_reference', workspace_id, environment_id, id
      FROM document_renders WHERE artifact_object_key_ciphertext IS NOT NULL;

CREATE OR REPLACE FUNCTION register_document_artifact_job_reference() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    artifact_upload_id text;
    artifact_render_id text;
    artifact_render_purpose text;
    artifact_render_state text;
BEGIN
    artifact_upload_id := NEW.payload->'content'->>'upload_id';
    SELECT upload.source_document_render_id, render.purpose, render.state
      INTO artifact_render_id, artifact_render_purpose, artifact_render_state
      FROM uploads upload
      JOIN document_renders render
        ON render.id = upload.source_document_render_id
       AND render.workspace_id = upload.workspace_id
       AND render.environment_id = upload.environment_id
     WHERE upload.id = artifact_upload_id
       AND upload.workspace_id = NEW.workspace_id
       AND upload.environment_id = NEW.environment_id
       AND upload.state = 'complete';
    IF artifact_render_id IS NOT NULL THEN
        IF artifact_render_purpose <> 'printable' OR artifact_render_state <> 'completed' THEN
            RAISE EXCEPTION 'document artifact is not printable';
        END IF;
        INSERT INTO document_artifact_job_references (
            workspace_id, environment_id, render_id, upload_id, job_id, retained_until
        ) VALUES (
            NEW.workspace_id, NEW.environment_id, artifact_render_id,
            artifact_upload_id, NEW.id, NEW.expires_at
        );
        UPDATE document_renders
           SET expires_at = GREATEST(expires_at, NEW.expires_at), updated_at = now()
         WHERE id = artifact_render_id
           AND workspace_id = NEW.workspace_id
           AND environment_id = NEW.environment_id
           AND state = 'completed'
           AND purpose = 'printable';
        IF NOT FOUND THEN
            RAISE EXCEPTION 'document artifact is not printable';
        END IF;
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION guard_printable_document_artifact_upload() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.source_document_render_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM document_renders render
            WHERE render.id = NEW.source_document_render_id
              AND render.workspace_id = NEW.workspace_id
              AND render.environment_id = NEW.environment_id
              AND render.purpose = 'printable'
              AND render.state = 'completed'
       ) THEN
        RAISE EXCEPTION 'document artifact is not printable';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER uploads_guard_printable_document_artifact
BEFORE INSERT OR UPDATE OF source_document_render_id ON uploads
FOR EACH ROW EXECUTE FUNCTION guard_printable_document_artifact_upload();

CREATE FUNCTION guard_printable_document_preview() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM document_renders render
         WHERE render.id = NEW.render_id
           AND render.workspace_id = NEW.workspace_id
           AND render.environment_id = NEW.environment_id
           AND render.purpose = 'printable'
           AND render.state = 'completed'
    ) THEN
        RAISE EXCEPTION 'document render cannot enter approval';
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER document_previews_guard_printable_render
BEFORE INSERT OR UPDATE OF render_id ON document_previews
FOR EACH ROW EXECUTE FUNCTION guard_printable_document_preview();
