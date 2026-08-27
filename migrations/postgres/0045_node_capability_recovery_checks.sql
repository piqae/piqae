-- Persist bounded capability-recovery progress independently of process
-- lifetime. Recently checked jobs are deferred so a permanently incompatible
-- prefix cannot starve later jobs whose node capability has been restored.
CREATE TABLE node_capability_recovery_checks (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    agent_id text NOT NULL,
    job_id text NOT NULL,
    next_check_at timestamptz NOT NULL DEFAULT now(),
    checked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, agent_id, job_id),
    FOREIGN KEY (workspace_id, environment_id)
        REFERENCES environments(workspace_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, agent_id)
        REFERENCES agents(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, job_id)
        REFERENCES jobs(workspace_id, environment_id, id) ON DELETE CASCADE,
    CHECK (checked_at IS NULL OR next_check_at >= checked_at)
);

CREATE INDEX node_capability_recovery_checks_due_idx
    ON node_capability_recovery_checks (
        workspace_id, environment_id, agent_id, next_check_at, job_id
    );
