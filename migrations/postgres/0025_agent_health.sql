ALTER TABLE agents
    ADD COLUMN health_started_at timestamptz,
    ADD COLUMN health_observed_at timestamptz,
    ADD COLUMN sqlite_integrity_ok boolean,
    ADD COLUMN executor_crashes bigint NOT NULL DEFAULT 0 CHECK (executor_crashes >= 0),
    ADD COLUMN last_error_code text;
