-- Queue content-free wake notifications in the same durable boundary as a
-- waiting job transition. The reconciliation marker is deliberately separate
-- from node_wake_hints: hints never own or expose a job identifier.

CREATE TABLE job_wake_reconciliations (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    job_id text NOT NULL,
    state_sequence bigint NOT NULL CHECK (state_sequence > 0),
    candidate_count integer NOT NULL CHECK (candidate_count BETWEEN 0 AND 16),
    reconciled_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, job_id, state_sequence),
    FOREIGN KEY (workspace_id, environment_id, job_id)
        REFERENCES jobs(workspace_id, environment_id, id) ON DELETE CASCADE
);

-- A hint has one logical outbox item. A worker may publish that item more than
-- once after a crash, but every delivery carries the same opaque hint ID.
CREATE UNIQUE INDEX routing_outbox_one_node_wake_hint_idx
    ON routing_outbox (workspace_id, environment_id, event_type, aggregate_id)
    WHERE aggregate_type = 'node_wake_hint'
      AND event_type = 'node.wake_hint.requested';

CREATE INDEX jobs_waiting_wake_repair_idx
    ON jobs (updated_at, id)
    WHERE state = 'waiting_for_agent' AND final_at IS NULL;
