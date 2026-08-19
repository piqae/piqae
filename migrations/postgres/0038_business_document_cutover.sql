-- Pre-release hard cutover to piqae.business-document/v1.
--
-- There is deliberately no document-v1 or hosted-adapter compatibility path.
-- Only prerelease document-subsystem data is reset. Existing accounts,
-- environments, nodes, uploads and print jobs remain intact. Uploads that were
-- sourced from an old render are detached before those render rows are removed.

DROP TRIGGER IF EXISTS jobs_register_document_artifact_reference ON jobs;
DROP TRIGGER IF EXISTS jobs_release_document_artifact_reference ON jobs;

DELETE FROM document_previews;
DELETE FROM document_artifact_job_references;
UPDATE uploads SET source_document_render_id = NULL
 WHERE source_document_render_id IS NOT NULL;
DELETE FROM document_conversions;
DELETE FROM document_renders;
DELETE FROM document_template_revisions;
DELETE FROM document_templates;

DROP VIEW document_encryption_key_references;
DROP TABLE document_conversions;

ALTER TABLE document_template_revisions
    DROP CONSTRAINT document_template_revisions_renderer_profile_check;
ALTER TABLE document_template_revisions
    ADD CONSTRAINT document_template_revisions_renderer_profile_check
    CHECK (renderer_profile = 'piqae.business-document/v1');

CREATE OR REPLACE VIEW document_encryption_key_references AS
    SELECT document_ciphertext_key_id(draft_ciphertext) AS key_id,
           'template_draft'::text AS resource_type,
           workspace_id, environment_id, id AS resource_id
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
      FROM document_renders WHERE artifact_object_key_ciphertext IS NOT NULL;

CREATE TRIGGER jobs_register_document_artifact_reference
AFTER INSERT ON jobs FOR EACH ROW
EXECUTE FUNCTION register_document_artifact_job_reference();
CREATE TRIGGER jobs_release_document_artifact_reference
AFTER UPDATE OF final_at ON jobs FOR EACH ROW
EXECUTE FUNCTION release_document_artifact_job_reference();
