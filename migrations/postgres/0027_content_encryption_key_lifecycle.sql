ALTER TABLE node_content_encryption_keys
    ADD COLUMN lifecycle_state text NOT NULL DEFAULT 'active',
    ADD COLUMN state_changed_at timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN destroyed_at timestamptz,
    ADD CONSTRAINT node_content_encryption_keys_lifecycle_check
        CHECK (lifecycle_state IN ('active', 'decrypt_only', 'revoked', 'destroyed')),
    ADD CONSTRAINT node_content_encryption_keys_destroyed_check
        CHECK ((lifecycle_state = 'destroyed') = (destroyed_at IS NOT NULL));

UPDATE node_content_encryption_keys
SET lifecycle_state = 'revoked', state_changed_at = revoked_at
WHERE revoked_at IS NOT NULL;

DROP INDEX node_content_encryption_keys_active_idx;
DROP INDEX node_content_encryption_keys_lookup_idx;

CREATE UNIQUE INDEX node_content_encryption_keys_active_idx
    ON node_content_encryption_keys (workspace_id, environment_id, agent_id)
    WHERE lifecycle_state = 'active';

CREATE INDEX node_content_encryption_keys_lookup_idx
    ON node_content_encryption_keys (workspace_id, environment_id, agent_id, key_id)
    WHERE lifecycle_state IN ('active', 'decrypt_only');

CREATE TABLE encrypted_job_key_references (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    agent_id text NOT NULL,
    key_id text NOT NULL,
    job_id text NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, job_id, key_id),
    FOREIGN KEY (workspace_id, environment_id, agent_id, key_id)
        REFERENCES node_content_encryption_keys(workspace_id, environment_id, agent_id, key_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (agent_id, workspace_id, environment_id)
        REFERENCES agents(id, workspace_id, environment_id) ON DELETE CASCADE
);

CREATE INDEX encrypted_job_key_references_key_idx
    ON encrypted_job_key_references (workspace_id, environment_id, agent_id, key_id);

CREATE FUNCTION guard_content_encryption_key_lifecycle() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.lifecycle_state = 'destroyed' AND NEW.lifecycle_state <> 'destroyed' THEN
        RAISE EXCEPTION 'destroyed content encryption keys cannot be resurrected';
    END IF;
    IF OLD.lifecycle_state = 'revoked' AND NEW.lifecycle_state IN ('active', 'decrypt_only') THEN
        RAISE EXCEPTION 'revoked content encryption keys cannot be resurrected';
    END IF;
    IF OLD.lifecycle_state = 'decrypt_only' AND NEW.lifecycle_state = 'active' THEN
        RAISE EXCEPTION 'decrypt-only content encryption keys cannot be reactivated';
    END IF;
    IF NEW.lifecycle_state IN ('revoked', 'destroyed') AND EXISTS (
        SELECT 1 FROM encrypted_job_key_references AS reference
        WHERE reference.workspace_id = OLD.workspace_id
          AND reference.environment_id = OLD.environment_id
          AND reference.agent_id = OLD.agent_id
          AND reference.key_id = OLD.key_id
    ) THEN
        RAISE EXCEPTION 'referenced content encryption keys cannot be revoked or destroyed';
    END IF;
    IF NEW.algorithm <> OLD.algorithm OR NEW.public_key_spki <> OLD.public_key_spki THEN
        RAISE EXCEPTION 'content encryption key identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER node_content_encryption_keys_lifecycle_guard
BEFORE UPDATE ON node_content_encryption_keys
FOR EACH ROW EXECUTE FUNCTION guard_content_encryption_key_lifecycle();
