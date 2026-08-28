ALTER TABLE job_acceptances
    ADD COLUMN route_reservation_id text,
    ADD COLUMN route_generation bigint,
    ADD COLUMN route_fencing_token_hash bytea,
    ADD COLUMN connector_generation bigint;

-- Existing prerelease rows may contain a malformed tenant projection, so the
-- constraints are installed NOT VALID: PostgreSQL enforces them for every new
-- or tenant-key-changing N/N-1 write without making the additive upgrade fail.
-- Historical rows remain visible for the conservative sweep/quarantine below.
ALTER TABLE job_acceptances
    ADD CONSTRAINT job_acceptances_job_tenant_fk
        FOREIGN KEY (workspace_id, environment_id, job_id)
        REFERENCES jobs (workspace_id, environment_id, id)
        ON DELETE CASCADE NOT VALID,
    ADD CONSTRAINT job_acceptances_agent_tenant_fk
        FOREIGN KEY (workspace_id, environment_id, agent_id)
        REFERENCES agents (workspace_id, environment_id, id)
        NOT VALID;

ALTER TABLE node_connectors
    ADD COLUMN admission_generation bigint NOT NULL DEFAULT 1
        CHECK (admission_generation > 0);

CREATE FUNCTION advance_node_connector_admission_generation()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.revoked_at IS NULL
       AND NEW.revoked_at IS NOT NULL
       AND current_setting('piqae.acceptance_aware_revoke', true) IS DISTINCT FROM 'on'
       AND EXISTS (
           SELECT 1
           FROM job_acceptances AS acceptance
           JOIN jobs AS job
             ON job.workspace_id = acceptance.workspace_id
            AND job.environment_id = acceptance.environment_id
            AND job.id = acceptance.job_id
           WHERE acceptance.workspace_id = OLD.workspace_id
             AND acceptance.environment_id = OLD.environment_id
             AND acceptance.agent_id = OLD.agent_id
             AND job.final_at IS NULL
       ) THEN
        RAISE EXCEPTION 'acceptance-aware connector revocation required'
            USING ERRCODE = '23514';
    END IF;
    IF OLD.revoked_at IS NOT NULL
       AND NEW.revoked_at IS NULL
       AND NEW.admission_generation = OLD.admission_generation THEN
        NEW.admission_generation := OLD.admission_generation + 1;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER node_connectors_advance_admission_generation
BEFORE UPDATE OF revoked_at ON node_connectors
FOR EACH ROW
EXECUTE FUNCTION advance_node_connector_admission_generation();

-- Mixed-version deployments must also fence an N-1 writer which does not know
-- about connector generations. Locking the same connector row serializes an
-- old acceptance INSERT with revoke: insert-first is swept by revoke, while
-- revoke-first aborts the old transaction before it can strand agent_accepted.
CREATE FUNCTION fence_job_acceptance_connector()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    agent_revoked_at timestamptz;
    connector_revoked_at timestamptz;
    connector_admission_generation bigint;
BEGIN
    SELECT revoked_at
      INTO agent_revoked_at
      FROM agents
     WHERE workspace_id = NEW.workspace_id
       AND environment_id = NEW.environment_id
       AND id = NEW.agent_id
     FOR UPDATE;

    IF NOT FOUND OR agent_revoked_at IS NOT NULL THEN
        RAISE EXCEPTION 'agent is revoked' USING ERRCODE = '23514';
    END IF;

    SELECT revoked_at, admission_generation
      INTO connector_revoked_at, connector_admission_generation
      FROM node_connectors
     WHERE workspace_id = NEW.workspace_id
       AND environment_id = NEW.environment_id
       AND agent_id = NEW.agent_id
     ORDER BY created_at DESC
     LIMIT 1
     FOR UPDATE;

    IF FOUND THEN
        IF connector_revoked_at IS NOT NULL THEN
            RAISE EXCEPTION 'connector is revoked' USING ERRCODE = '23514';
        END IF;
        IF NEW.connector_generation IS NULL THEN
            NEW.connector_generation := connector_admission_generation;
        ELSIF NEW.connector_generation <> connector_admission_generation THEN
            RAISE EXCEPTION 'connector generation is stale' USING ERRCODE = '23514';
        END IF;
    ELSIF NEW.connector_generation IS NULL THEN
        NEW.connector_generation := 1;
    END IF;
    RETURN NEW;
END;
$$;

-- Before connector revocation became acceptance-aware, an offline connector
-- could be revoked while a job remained `agent_accepted`.  Such a row cannot
-- safely be reactivated after upgrade: the node may or may not have crossed
-- the native handoff boundary.  Fence these historical rows conservatively and
-- keep every delivery projection consistent with the job terminal state.
CREATE TEMP TABLE migration_0043_revoked_acceptances ON COMMIT DROP AS
SELECT DISTINCT ON (job.workspace_id, job.environment_id, job.id)
    job.workspace_id,
    job.environment_id,
    job.id AS job_id,
    job.agent_id,
    job.state_sequence + 1 AS event_sequence,
    now() AS terminalized_at
FROM jobs AS job
JOIN job_acceptances AS acceptance
  ON acceptance.workspace_id = job.workspace_id
 AND acceptance.environment_id = job.environment_id
 AND acceptance.job_id = job.id
 AND acceptance.agent_id = job.agent_id
JOIN node_connectors AS connector
  ON connector.workspace_id = acceptance.workspace_id
 AND connector.environment_id = acceptance.environment_id
 AND connector.agent_id = acceptance.agent_id
WHERE connector.revoked_at IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM node_connectors AS active_connector
      WHERE active_connector.workspace_id = acceptance.workspace_id
        AND active_connector.environment_id = acceptance.environment_id
        AND active_connector.agent_id = acceptance.agent_id
        AND active_connector.revoked_at IS NULL
  )
  AND job.final_at IS NULL
ORDER BY job.workspace_id, job.environment_id, job.id, connector.revoked_at DESC;

UPDATE physical_destinations AS destination
SET state = 'attention', updated_at = affected.terminalized_at
FROM migration_0043_revoked_acceptances AS affected
WHERE destination.workspace_id = affected.workspace_id
  AND destination.environment_id = affected.environment_id
  AND destination.id IN (
      SELECT attempt.destination_id
      FROM delivery_attempts AS attempt
      WHERE attempt.workspace_id = affected.workspace_id
        AND attempt.environment_id = affected.environment_id
        AND attempt.job_id = affected.job_id
        AND attempt.final_at IS NULL
  );

UPDATE delivery_attempts AS attempt
SET state = 'delivery_uncertain',
    final_at = affected.terminalized_at,
    updated_at = affected.terminalized_at
FROM migration_0043_revoked_acceptances AS affected
WHERE attempt.workspace_id = affected.workspace_id
  AND attempt.environment_id = affected.environment_id
  AND attempt.job_id = affected.job_id
  AND attempt.final_at IS NULL;

UPDATE route_reservations AS reservation
SET state = 'released',
    released_at = affected.terminalized_at,
    updated_at = affected.terminalized_at
FROM migration_0043_revoked_acceptances AS affected
WHERE reservation.workspace_id = affected.workspace_id
  AND reservation.environment_id = affected.environment_id
  AND reservation.job_id = affected.job_id
  AND reservation.state = 'active';

INSERT INTO job_events (
    id, workspace_id, environment_id, job_id, sequence, state, payload, occurred_at
)
SELECT
    'evt_0' || substring(md5(affected.workspace_id || ':' || affected.environment_id || ':' || affected.job_id) from 1 for 25),
    affected.workspace_id,
    affected.environment_id,
    affected.job_id,
    affected.event_sequence,
    'delivery_uncertain',
    jsonb_build_object(
        'id', '0' || substring(md5(affected.workspace_id || ':' || affected.environment_id || ':' || affected.job_id) from 1 for 25),
        'job_id', regexp_replace(affected.job_id, '^job_', ''),
        'sequence', affected.event_sequence,
        'state', 'delivery_uncertain',
        'reason', 'ambiguous_handoff',
        'message', 'Connector was revoked before acceptance recovery was available; delivery is uncertain',
        'agent_id', regexp_replace(affected.agent_id, '^agt_', ''),
        'native_job_id', NULL,
        'occurred_at', affected.terminalized_at
    ),
    affected.terminalized_at
FROM migration_0043_revoked_acceptances AS affected;

UPDATE jobs AS job
SET state = 'delivery_uncertain',
    state_sequence = affected.event_sequence,
    final_at = affected.terminalized_at,
    delivery_uncertain_since = COALESCE(job.delivery_uncertain_since, affected.terminalized_at),
    payload = jsonb_set(
        jsonb_set(job.payload, '{state}', '"delivery_uncertain"'::jsonb, true),
        '{delivery_uncertain_since}', to_jsonb(affected.terminalized_at), true
    ),
    updated_at = affected.terminalized_at
FROM migration_0043_revoked_acceptances AS affected
WHERE job.workspace_id = affected.workspace_id
  AND job.environment_id = affected.environment_id
  AND job.id = affected.job_id;

WITH proofs AS (
    SELECT DISTINCT ON (
        reservation.workspace_id,
        reservation.environment_id,
        reservation.job_id,
        route.agent_id
    )
        reservation.workspace_id,
        reservation.environment_id,
        reservation.job_id,
        route.agent_id,
        reservation.id AS reservation_id,
        reservation.generation,
        reservation.fencing_token_hash
    FROM route_reservations AS reservation
    JOIN delivery_attempts AS attempt
      ON attempt.workspace_id = reservation.workspace_id
     AND attempt.environment_id = reservation.environment_id
     AND attempt.id = reservation.attempt_id
    JOIN printer_routes AS route
      ON route.workspace_id = attempt.workspace_id
     AND route.environment_id = attempt.environment_id
     AND route.id = attempt.route_id
    WHERE attempt.state IN (
        'queued_local', 'handing_to_spooler', 'accepted_by_spooler',
        'printing_reported', 'completed_reported', 'delivery_uncertain'
    )
    ORDER BY reservation.workspace_id, reservation.environment_id,
             reservation.job_id, route.agent_id, reservation.created_at DESC
)
UPDATE job_acceptances AS acceptance
SET route_reservation_id = proof.reservation_id,
    route_generation = proof.generation,
    route_fencing_token_hash = decode(proof.fencing_token_hash, 'hex')
FROM proofs AS proof
WHERE acceptance.workspace_id = proof.workspace_id
  AND acceptance.environment_id = proof.environment_id
  AND acceptance.job_id = proof.job_id
  AND acceptance.agent_id = proof.agent_id;

UPDATE job_acceptances AS acceptance
SET connector_generation = connector.admission_generation
FROM node_connectors AS connector
WHERE acceptance.workspace_id = connector.workspace_id
  AND acceptance.environment_id = connector.environment_id
  AND acceptance.agent_id = connector.agent_id;

ALTER TABLE job_acceptances
    ADD CONSTRAINT job_acceptances_route_proof_complete CHECK (COALESCE((
        (
            route_reservation_id IS NULL
            AND route_generation IS NULL
            AND route_fencing_token_hash IS NULL
        ) OR (
            route_reservation_id IS NOT NULL
            AND length(trim(route_reservation_id)) > 0
            AND route_generation > 0
            AND route_fencing_token_hash IS NOT NULL
            AND octet_length(route_fencing_token_hash) = 32
        )
    ), false));

ALTER TABLE job_acceptances
    ADD CONSTRAINT job_acceptances_connector_generation_valid CHECK (
        connector_generation IS NULL OR connector_generation > 0
    );

-- Revocation terminalization and tenant notification must commit atomically.
-- A nullable key preserves legacy event insertion while allowing a retry to
-- recover the original, time-sortable event ID instead of duplicating it.
ALTER TABLE webhook_events
    ADD COLUMN idempotency_key text;

CREATE UNIQUE INDEX webhook_events_tenant_idempotency_unique
    ON webhook_events (workspace_id, environment_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE TRIGGER job_acceptances_fence_connector
BEFORE INSERT OR UPDATE OF connector_generation ON job_acceptances
FOR EACH ROW
EXECUTE FUNCTION fence_job_acceptance_connector();
