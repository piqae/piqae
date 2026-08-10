-- Versioned document encryption-key lifecycle and persistent reference audit.
-- Key material never enters PostgreSQL; ciphertext carries its non-secret key id.

ALTER TABLE document_templates DROP CONSTRAINT document_templates_draft_ciphertext_check;
ALTER TABLE document_templates ADD CONSTRAINT document_templates_draft_ciphertext_check
    CHECK (octet_length(draft_ciphertext) BETWEEN 29 AND 1048704);
ALTER TABLE document_template_revisions
    DROP CONSTRAINT document_template_revisions_spec_ciphertext_check;
ALTER TABLE document_template_revisions
    ADD CONSTRAINT document_template_revisions_spec_ciphertext_check
    CHECK (octet_length(spec_ciphertext) BETWEEN 29 AND 1048704);
ALTER TABLE document_renders DROP CONSTRAINT document_renders_input_ciphertext_check;
ALTER TABLE document_renders ADD CONSTRAINT document_renders_input_ciphertext_check
    CHECK (octet_length(input_ciphertext) BETWEEN 29 AND 1048704);
ALTER TABLE document_renders DROP CONSTRAINT document_renders_artifact_object_key_ciphertext_check;
ALTER TABLE document_renders ADD CONSTRAINT document_renders_artifact_object_key_ciphertext_check
    CHECK (artifact_object_key_ciphertext IS NULL
           OR octet_length(artifact_object_key_ciphertext) BETWEEN 29 AND 1120);

CREATE TABLE document_encryption_keys (
    key_id text PRIMARY KEY CHECK (key_id ~ '^[A-Za-z0-9_.-]{1,64}$'),
    lifecycle_state text NOT NULL CHECK (lifecycle_state IN ('standby', 'active', 'decrypt_only', 'retired')),
    first_seen_at timestamptz NOT NULL DEFAULT now(),
    state_changed_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX document_encryption_keys_one_active_idx
    ON document_encryption_keys ((lifecycle_state)) WHERE lifecycle_state = 'active';

-- Ciphertexts written before this migration are nonce || ciphertext and use
-- the reserved legacy-v1 id. v2 is PDOC || version || id-length || id || nonce || ciphertext.
CREATE FUNCTION document_ciphertext_key_id(ciphertext bytea) RETURNS text
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path = pg_catalog AS $$
DECLARE
    key_length integer;
    key_identifier text;
BEGIN
    IF octet_length(ciphertext) >= 18
       AND substring(ciphertext FROM 1 FOR 4) = decode('50444f43', 'hex')
       AND get_byte(ciphertext, 4) = 2 THEN
        key_length := get_byte(ciphertext, 5);
        IF key_length < 1 OR key_length > 64 OR octet_length(ciphertext) < 18 + key_length THEN
            RAISE EXCEPTION 'malformed versioned document ciphertext';
        END IF;
        key_identifier := convert_from(substring(ciphertext FROM 7 FOR key_length), 'UTF8');
        IF key_identifier !~ '^[A-Za-z0-9_.-]{1,64}$' THEN
            RAISE EXCEPTION 'invalid document ciphertext key id';
        END IF;
        RETURN key_identifier;
    END IF;
    RETURN 'legacy-v1';
END;
$$;

CREATE VIEW document_encryption_key_references AS
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
      FROM document_renders WHERE artifact_object_key_ciphertext IS NOT NULL;

CREATE FUNCTION guard_document_encryption_key_retirement() RETURNS trigger
LANGUAGE plpgsql SET search_path FROM CURRENT AS $$
BEGIN
    IF NEW.lifecycle_state = 'retired' AND OLD.lifecycle_state <> 'retired'
       AND EXISTS (
           SELECT 1 FROM document_encryption_key_references AS reference
            WHERE reference.key_id = OLD.key_id
       ) THEN
        RAISE EXCEPTION 'referenced document encryption keys cannot be retired';
    END IF;
    IF OLD.lifecycle_state = 'retired' AND NEW.lifecycle_state <> 'retired' THEN
        RAISE EXCEPTION 'retired document encryption keys cannot be reactivated';
    END IF;
    IF OLD.lifecycle_state = 'decrypt_only' AND NEW.lifecycle_state = 'active' THEN
        RAISE EXCEPTION 'decrypt-only document encryption keys cannot be reactivated';
    END IF;
    IF OLD.key_id <> NEW.key_id THEN
        RAISE EXCEPTION 'document encryption key identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER document_encryption_keys_lifecycle_guard
BEFORE UPDATE ON document_encryption_keys
FOR EACH ROW EXECUTE FUNCTION guard_document_encryption_key_retirement();

CREATE FUNCTION guard_document_encryption_key_deletion() RETURNS trigger
LANGUAGE plpgsql SET search_path FROM CURRENT AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM document_encryption_key_references AS reference
         WHERE reference.key_id = OLD.key_id
    ) THEN
        RAISE EXCEPTION 'referenced document encryption keys cannot be deleted';
    END IF;
    RETURN OLD;
END;
$$;

CREATE TRIGGER document_encryption_keys_delete_guard
BEFORE DELETE ON document_encryption_keys
FOR EACH ROW EXECUTE FUNCTION guard_document_encryption_key_deletion();

-- All existing v1 ciphertext is explicitly accounted for before application rotation.
INSERT INTO document_encryption_keys (key_id, lifecycle_state)
VALUES ('legacy-v1', 'active');
