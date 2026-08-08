CREATE FUNCTION guard_encrypted_job_key_reference() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    key_state text;
BEGIN
    SELECT lifecycle_state INTO key_state
    FROM node_content_encryption_keys
    WHERE workspace_id = NEW.workspace_id
      AND environment_id = NEW.environment_id
      AND agent_id = NEW.agent_id
      AND key_id = NEW.key_id
    FOR UPDATE;

    IF key_state IS NULL OR key_state NOT IN ('active', 'decrypt_only') THEN
        RAISE EXCEPTION 'encrypted jobs require an active or decrypt-only recipient key';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER encrypted_job_key_references_lifecycle_guard
BEFORE INSERT ON encrypted_job_key_references
FOR EACH ROW EXECUTE FUNCTION guard_encrypted_job_key_reference();
