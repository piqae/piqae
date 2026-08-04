-- Bill new Cloud usage only when a Live job is reported complete.
--
-- Existing print_job_accepted rows are immutable historical billing evidence.
-- They remain countable, but the cross-kind unique index prevents a job that
-- crossed the cutover from being charged a second time at completion.

DROP TRIGGER usage_ledger_live_acceptance_guard ON usage_ledger;
DROP FUNCTION enforce_live_accepted_job_usage();

DROP INDEX usage_one_acceptance_per_job_idx;

CREATE UNIQUE INDEX usage_one_billable_print_per_job_idx
    ON usage_ledger (job_id)
    WHERE kind IN ('print_job_accepted', 'print_job_reported_complete')
      AND job_id IS NOT NULL;

CREATE FUNCTION enforce_live_billable_job_usage()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.kind IN ('print_job_accepted', 'print_job_reported_complete') THEN
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
            RAISE EXCEPTION 'billable print usage must reference its Live tenant job';
        END IF;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER usage_ledger_live_billable_job_guard
    BEFORE INSERT ON usage_ledger
    FOR EACH ROW EXECUTE FUNCTION enforce_live_billable_job_usage();

COMMENT ON TABLE usage_ledger IS
    'Immutable billable events. New usage is counted once when a Live job is reported complete; legacy accepted rows are retained for audit continuity. A completion report is not proof that ink reached paper.';
COMMENT ON COLUMN workspace_entitlements.job_overage_cents IS
    'Pro overage is USD 0.25 per additional 1000 reported-complete Live jobs; Free has no overage.';
