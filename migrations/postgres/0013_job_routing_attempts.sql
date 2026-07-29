CREATE TABLE job_routing_attempts (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    job_id text NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    target_id text NOT NULL,
    from_binding_id text,
    to_binding_id text NOT NULL,
    from_agent_id text NOT NULL,
    to_agent_id text NOT NULL,
    from_printer_id text NOT NULL,
    to_printer_id text NOT NULL,
    reason text NOT NULL CHECK (reason IN ('node_recovered', 'standby_recovery')),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX job_routing_attempts_job_idx
    ON job_routing_attempts (workspace_id, environment_id, job_id, created_at, id);
