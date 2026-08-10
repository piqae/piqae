-- Share immutable render artifacts with print jobs without copying bytes.
-- Tenant columns are repeated on every edge so cross-tenant references cannot
-- be represented, even if an opaque identifier is guessed.
ALTER TABLE uploads DROP CONSTRAINT uploads_object_key_key;
ALTER TABLE uploads
    ADD COLUMN source_document_render_id text,
    ADD COLUMN acquisition_sha256 text
        CHECK (acquisition_sha256 IS NULL OR acquisition_sha256 ~ '^[0-9a-f]{64}$');

CREATE UNIQUE INDEX document_renders_tenant_identity_idx
    ON document_renders (workspace_id, environment_id, id);
CREATE UNIQUE INDEX jobs_tenant_identity_idx
    ON jobs (workspace_id, environment_id, id);
CREATE UNIQUE INDEX uploads_tenant_identity_idx
    ON uploads (workspace_id, environment_id, id);

ALTER TABLE uploads ADD CONSTRAINT uploads_source_document_render_fk
    FOREIGN KEY (workspace_id, environment_id, source_document_render_id)
    REFERENCES document_renders (workspace_id, environment_id, id)
    ON DELETE RESTRICT;
CREATE UNIQUE INDEX uploads_document_artifact_acquisition_idx
    ON uploads (workspace_id, environment_id, source_document_render_id, acquisition_sha256)
    WHERE source_document_render_id IS NOT NULL;

CREATE TABLE document_artifact_job_references (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    render_id text NOT NULL,
    upload_id text NOT NULL,
    job_id text NOT NULL,
    retained_until timestamptz NOT NULL,
    released_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, job_id),
    FOREIGN KEY (workspace_id, environment_id, render_id)
        REFERENCES document_renders (workspace_id, environment_id, id) ON DELETE RESTRICT,
    FOREIGN KEY (workspace_id, environment_id, job_id)
        REFERENCES jobs (workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, upload_id)
        REFERENCES uploads (workspace_id, environment_id, id) ON DELETE RESTRICT,
    CHECK (retained_until > created_at),
    CHECK (released_at IS NULL OR released_at >= created_at)
);
CREATE INDEX document_artifact_active_references_idx
    ON document_artifact_job_references
       (workspace_id, environment_id, render_id, retained_until)
    WHERE released_at IS NULL;

CREATE FUNCTION register_document_artifact_job_reference() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    artifact_upload_id text;
    artifact_render_id text;
BEGIN
    artifact_upload_id := NEW.payload->'content'->>'upload_id';
    SELECT source_document_render_id INTO artifact_render_id
      FROM uploads
     WHERE id = artifact_upload_id
       AND workspace_id = NEW.workspace_id
       AND environment_id = NEW.environment_id
       AND state = 'complete';
    IF artifact_render_id IS NOT NULL THEN
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
           AND state = 'completed';
        IF NOT FOUND THEN
            RAISE EXCEPTION 'document artifact is not printable';
        END IF;
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER jobs_register_document_artifact_reference
AFTER INSERT ON jobs FOR EACH ROW
EXECUTE FUNCTION register_document_artifact_job_reference();

CREATE FUNCTION release_document_artifact_job_reference() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.final_at IS NULL AND NEW.final_at IS NOT NULL THEN
        UPDATE document_artifact_job_references
           SET released_at = now()
         WHERE workspace_id = NEW.workspace_id
           AND environment_id = NEW.environment_id
           AND job_id = NEW.id
           AND released_at IS NULL;
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER jobs_release_document_artifact_reference
AFTER UPDATE OF final_at ON jobs FOR EACH ROW
EXECUTE FUNCTION release_document_artifact_job_reference();
