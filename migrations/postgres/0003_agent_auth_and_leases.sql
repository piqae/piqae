ALTER TABLE jobs
    ADD COLUMN lease_id uuid,
    ADD COLUMN lease_token_hash bytea;

CREATE TABLE agent_nonces (
    agent_id text NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    nonce text NOT NULL,
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (agent_id, nonce)
);

CREATE INDEX agent_nonces_expiry_idx ON agent_nonces (expires_at);
CREATE INDEX enrolment_tokens_secret_idx
    ON enrolment_tokens (secret_hash)
    WHERE consumed_at IS NULL;

CREATE TABLE job_acceptances (
    job_id text PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    agent_id text NOT NULL REFERENCES agents(id),
    lease_id uuid NOT NULL,
    content_sha256 text,
    local_sequence bigint NOT NULL CHECK (local_sequence >= 0),
    accepted_at timestamptz NOT NULL DEFAULT now()
);
