-- Fill the intentionally reserved version 0012 with the canonical hosted
-- Free/Pro projection. SQLx applies this migration to both fresh databases and
-- installations that already recorded versions 0013-0015.

ALTER TABLE billing_subscriptions
    ADD COLUMN IF NOT EXISTS stripe_event_id text,
    ADD COLUMN IF NOT EXISTS stripe_event_created_at timestamptz,
    ADD COLUMN IF NOT EXISTS billing_interval text NOT NULL DEFAULT 'monthly'
        CHECK (billing_interval IN ('monthly', 'annual'));

ALTER TABLE billing_subscriptions
    DROP CONSTRAINT IF EXISTS billing_subscriptions_plan_check,
    DROP CONSTRAINT IF EXISTS billing_subscriptions_status_check;

UPDATE billing_subscriptions
SET plan = CASE WHEN plan = 'free' THEN 'free' ELSE 'pro' END;

UPDATE billing_subscriptions
SET status = CASE status
    WHEN 'canceled' THEN 'cancelled'
    WHEN 'incomplete' THEN 'past_due'
    WHEN 'incomplete_expired' THEN 'unpaid'
    ELSE status
END;

ALTER TABLE billing_subscriptions
    ADD CONSTRAINT billing_subscriptions_plan_check
        CHECK (plan IN ('free', 'pro')),
    ADD CONSTRAINT billing_subscriptions_status_check
        CHECK (status IN ('active', 'trialing', 'past_due', 'unpaid', 'paused', 'cancelled'));

ALTER TABLE workspace_entitlements
    ADD COLUMN IF NOT EXISTS job_overage_unit bigint
        CHECK (job_overage_unit IS NULL OR job_overage_unit > 0),
    ADD COLUMN IF NOT EXISTS job_overage_cents bigint
        CHECK (job_overage_cents IS NULL OR job_overage_cents >= 0);

ALTER TABLE workspace_entitlements
    DROP CONSTRAINT IF EXISTS workspace_entitlements_plan_check;

UPDATE workspace_entitlements
SET plan = CASE WHEN plan = 'free' THEN 'free' ELSE 'pro' END;

ALTER TABLE workspace_entitlements
    ADD CONSTRAINT workspace_entitlements_plan_check
        CHECK (plan IN ('free', 'pro'));

ALTER TABLE workspace_entitlements
    DROP COLUMN IF EXISTS included_active_tenants,
    DROP COLUMN IF EXISTS active_agent_overage_cents,
    DROP COLUMN IF EXISTS active_tenant_overage_cents;

UPDATE workspace_entitlements
SET
    included_jobs = CASE plan WHEN 'free' THEN 100 ELSE 25000 END,
    node_limit = CASE plan WHEN 'free' THEN 1 ELSE 25 END,
    metadata_retention_days = CASE plan WHEN 'free' THEN 7 ELSE 90 END,
    document_retention_hours = CASE plan WHEN 'free' THEN 24 ELSE 168 END,
    job_overage_unit = CASE plan WHEN 'pro' THEN 1000 ELSE NULL END,
    job_overage_cents = CASE plan WHEN 'pro' THEN 25 ELSE NULL END,
    updated_at = now();

INSERT INTO workspace_entitlements (
    workspace_id, plan, included_jobs, node_limit,
    metadata_retention_days, document_retention_hours,
    accept_new_cloud_jobs, job_overage_unit, job_overage_cents
)
SELECT
    workspace.id, 'free', 100, 1, 7, 24, true, NULL, NULL
FROM workspaces workspace
ON CONFLICT (workspace_id) DO NOTHING;

ALTER TABLE workspaces
    ADD COLUMN IF NOT EXISTS identity_provider text,
    ADD COLUMN IF NOT EXISTS identity_organization_id text,
    ADD CONSTRAINT workspaces_identity_pair_check CHECK (
        (identity_provider IS NULL) = (identity_organization_id IS NULL)
    ),
    ADD CONSTRAINT workspaces_identity_provider_check CHECK (
        identity_provider IS NULL OR identity_provider IN ('workos', 'oidc')
    );

UPDATE workspaces
SET
    identity_provider = 'workos',
    identity_organization_id = workos_organization_id
WHERE workos_organization_id IS NOT NULL;

CREATE UNIQUE INDEX workspaces_identity_organization_unique
    ON workspaces (identity_provider, identity_organization_id)
    WHERE identity_provider IS NOT NULL;

ALTER TABLE billing_webhook_receipts
    ADD COLUMN IF NOT EXISTS stripe_created_at timestamptz;

ALTER TABLE usage_exports
    ADD COLUMN IF NOT EXISTS claim_token text,
    ADD COLUMN IF NOT EXISTS claimed_at timestamptz,
    ADD COLUMN IF NOT EXISTS next_attempt_at timestamptz;

ALTER TABLE usage_exports
    DROP CONSTRAINT IF EXISTS usage_exports_state_check;

ALTER TABLE usage_exports
    ADD CONSTRAINT usage_exports_state_check
        CHECK (state IN ('pending', 'submitting', 'submitted', 'failed'));

CREATE INDEX usage_exports_dispatch_idx
    ON usage_exports (state, next_attempt_at, created_at)
    WHERE state IN ('pending', 'failed');

ALTER TABLE agents
    DROP CONSTRAINT IF EXISTS agents_workspace_id_environment_id_installation_id_key;

CREATE UNIQUE INDEX agents_active_installation_unique
    ON agents (workspace_id, environment_id, installation_id)
    WHERE revoked_at IS NULL;

ALTER TABLE device_authorizations
    ADD COLUMN enrolled_agent_id text REFERENCES agents(id) ON DELETE RESTRICT;

ALTER TABLE jobs
    ADD CONSTRAINT jobs_tenant_identity_unique
        UNIQUE (workspace_id, environment_id, id);

CREATE UNIQUE INDEX environments_tenant_identity_unique
    ON environments (workspace_id, id);

ALTER TABLE usage_ledger
    ADD CONSTRAINT usage_ledger_positive_units_check CHECK (units > 0),
    ADD CONSTRAINT usage_ledger_environment_tenant_fk
        FOREIGN KEY (workspace_id, environment_id)
        REFERENCES environments(workspace_id, id) ON DELETE RESTRICT,
    ADD CONSTRAINT usage_ledger_job_tenant_fk
        FOREIGN KEY (workspace_id, environment_id, job_id)
        REFERENCES jobs(workspace_id, environment_id, id) ON DELETE RESTRICT;

CREATE OR REPLACE FUNCTION enforce_live_accepted_job_usage()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.kind = 'print_job_accepted' THEN
        IF NEW.units <> 1 OR NEW.job_id IS NULL OR NOT EXISTS (
            SELECT 1
            FROM jobs job
            JOIN environments environment
              ON environment.id = job.environment_id
             AND environment.workspace_id = job.workspace_id
            WHERE job.id = NEW.job_id
              AND job.workspace_id = NEW.workspace_id
              AND job.environment_id = NEW.environment_id
              AND environment.kind = 'live'
        ) THEN
            RAISE EXCEPTION 'print_job_accepted usage must reference its Live tenant job';
        END IF;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER usage_ledger_live_acceptance_guard
    BEFORE INSERT ON usage_ledger
    FOR EACH ROW EXECUTE FUNCTION enforce_live_accepted_job_usage();

CREATE OR REPLACE FUNCTION prevent_usage_ledger_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'usage_ledger is immutable';
END
$$;

CREATE TRIGGER usage_ledger_immutable_update
    BEFORE UPDATE OR DELETE ON usage_ledger
    FOR EACH ROW EXECUTE FUNCTION prevent_usage_ledger_mutation();

COMMENT ON TABLE usage_ledger IS
    'Immutable billable events. A Live job is counted exactly once at OS spooler acceptance, not physical delivery.';
COMMENT ON COLUMN workspace_entitlements.included_jobs IS
    'Nominal allowance: Free 100 per UTC month and Pro 25000 per monthly period; annual Pro overrides this to 300000 per Stripe subscription period.';
COMMENT ON COLUMN workspace_entitlements.job_overage_cents IS
    'Pro overage is USD 0.25 per additional 1000 accepted Live jobs; Free has no overage.';
