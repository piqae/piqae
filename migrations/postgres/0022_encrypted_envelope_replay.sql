CREATE TABLE consumed_encrypted_envelopes (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    envelope_id text NOT NULL,
    manifest_sha256 text NOT NULL CHECK (length(manifest_sha256) = 64),
    job_id text NOT NULL,
    consumed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, envelope_id),
    FOREIGN KEY (job_id) REFERENCES jobs(id) ON DELETE CASCADE
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (envelope_id ~ '^env_[A-Za-z0-9_-]{20,255}$')
);

CREATE INDEX consumed_encrypted_envelopes_job_idx
    ON consumed_encrypted_envelopes (workspace_id, environment_id, job_id);
