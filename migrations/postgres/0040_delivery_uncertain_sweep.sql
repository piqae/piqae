-- Uncertain delivery is the one job outcome that cannot be proved either way,
-- so it needs its own durable anchor rather than being inferred from `final_at`
-- (which carries the agent-reported clock) or `updated_at` (which any later
-- write moves). `delivery_uncertain_since` is written by the control plane on
-- the transition into `delivery_uncertain`, using the server clock, and never
-- moves afterwards because the state machine has no transition out of it.
--
-- `delivery_uncertain_alerted_at` is the sweep's idempotency fence: a job is
-- surfaced as "still uncertain" at most once, and only a deliberate reset makes
-- it eligible again.
ALTER TABLE jobs
    ADD COLUMN delivery_uncertain_since timestamptz,
    ADD COLUMN delivery_uncertain_alerted_at timestamptz;

-- Existing uncertain jobs predate the anchor. `final_at` is the moment the job
-- became terminal, which for these rows is the moment it became uncertain, so
-- it is the closest truthful backfill. The alert fence is left NULL on purpose:
-- these jobs are exactly the ones nothing has ever noticed, and the sweep is
-- bounded per batch so they drain at a controlled rate instead of storming.
UPDATE jobs
   SET delivery_uncertain_since = COALESCE(final_at, updated_at, created_at)
 WHERE state = 'delivery_uncertain';

CREATE INDEX jobs_delivery_uncertain_unresolved_idx
    ON jobs (delivery_uncertain_since)
    WHERE state = 'delivery_uncertain' AND delivery_uncertain_alerted_at IS NULL;
