ALTER TABLE enrolment_tokens
    ADD COLUMN agent_id text REFERENCES agents(id) ON DELETE SET NULL;

CREATE INDEX enrolment_tokens_tenant_status_idx
    ON enrolment_tokens (workspace_id, environment_id, id);
