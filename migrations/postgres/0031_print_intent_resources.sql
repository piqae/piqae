-- Durable, tenant-scoped resources for capability-aware print intents.
-- Driver-native payloads remain in encrypted profile/job records; these tables
-- contain only display-safe, validated semantic projections and immutable digests.

ALTER TABLE stocks
    ADD COLUMN revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0);

ALTER TABLE printers
    ADD CONSTRAINT printers_tenant_identity_unique
    UNIQUE (workspace_id, environment_id, id);

ALTER TABLE printers
    ADD COLUMN semantic_capabilities jsonb NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(semantic_capabilities) = 'object');

ALTER TABLE stocks
    ADD CONSTRAINT stocks_tenant_identity_unique
    UNIQUE (workspace_id, environment_id, id);

CREATE TABLE printer_capability_documents (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    printer_id text NOT NULL,
    revision bigint NOT NULL CHECK (revision > 0),
    schema_version integer NOT NULL CHECK (schema_version = 1),
    driver_fingerprint_sha256 text NOT NULL CHECK (driver_fingerprint_sha256 ~ '^[0-9a-f]{64}$'),
    document jsonb NOT NULL CHECK (jsonb_typeof(document) = 'object'),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, printer_id, revision),
    FOREIGN KEY (workspace_id, environment_id, printer_id)
        REFERENCES printers(workspace_id, environment_id, id) ON DELETE CASCADE
);

CREATE INDEX printer_capability_documents_latest_idx
    ON printer_capability_documents (workspace_id, environment_id, printer_id, revision DESC);

CREATE TABLE stock_revisions (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    stock_id text NOT NULL,
    revision bigint NOT NULL CHECK (revision > 0),
    specification jsonb NOT NULL CHECK (jsonb_typeof(specification) = 'object'),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, stock_id, revision),
    FOREIGN KEY (workspace_id, environment_id, stock_id)
        REFERENCES stocks(workspace_id, environment_id, id) ON DELETE CASCADE
);

CREATE TABLE print_workflows (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 255),
    archived boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, environment_id, name),
    UNIQUE (workspace_id, environment_id, id)
);

CREATE INDEX print_workflows_tenant_idx
    ON print_workflows (workspace_id, environment_id, created_at, id);

CREATE TABLE print_workflow_revisions (
    workspace_id text NOT NULL,
    environment_id text NOT NULL,
    workflow_id text NOT NULL,
    revision bigint NOT NULL CHECK (revision > 0),
    printer_id text NOT NULL,
    capability_revision bigint NOT NULL CHECK (capability_revision > 0),
    profile_id text,
    profile_revision bigint CHECK (profile_revision > 0),
    stock_id text,
    stock_revision bigint CHECK (stock_revision > 0),
    definition jsonb NOT NULL CHECK (jsonb_typeof(definition) = 'object'),
    safe_overrides text[] NOT NULL DEFAULT '{}',
    published boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, workflow_id, revision),
    FOREIGN KEY (workspace_id, environment_id, workflow_id)
        REFERENCES print_workflows(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, printer_id)
        REFERENCES printers(workspace_id, environment_id, id),
    FOREIGN KEY (workspace_id, environment_id, stock_id)
        REFERENCES stocks(workspace_id, environment_id, id)
);

CREATE TABLE printer_loaded_media (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    printer_id text NOT NULL,
    source text NOT NULL DEFAULT 'default' CHECK (char_length(source) BETWEEN 1 AND 255),
    stock_id text,
    stock_revision bigint CHECK (stock_revision > 0),
    confidence text NOT NULL CHECK (confidence IN ('reported', 'operator_confirmed', 'inferred', 'unknown')),
    calibration_state text NOT NULL DEFAULT 'unknown'
        CHECK (calibration_state IN ('current', 'required', 'unknown')),
    remaining_amount jsonb CHECK (remaining_amount IS NULL OR jsonb_typeof(remaining_amount) = 'object'),
    observed_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, printer_id, source),
    FOREIGN KEY (workspace_id, environment_id, printer_id)
        REFERENCES printers(workspace_id, environment_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, environment_id, stock_id)
        REFERENCES stocks(workspace_id, environment_id, id)
);

CREATE TABLE resolved_print_tickets (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    environment_id text NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    digest text NOT NULL CHECK (digest ~ '^[0-9a-f]{64}$'),
    printer_id text NOT NULL,
    capability_revision bigint NOT NULL CHECK (capability_revision > 0),
    workflow_id text,
    workflow_revision bigint CHECK (workflow_revision > 0),
    stock_id text,
    stock_revision bigint CHECK (stock_revision > 0),
    display_ticket jsonb NOT NULL CHECK (jsonb_typeof(display_ticket) = 'object'),
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, environment_id, digest),
    FOREIGN KEY (workspace_id, environment_id, printer_id)
        REFERENCES printers(workspace_id, environment_id, id)
);

CREATE INDEX resolved_print_tickets_expiry_idx
    ON resolved_print_tickets (workspace_id, environment_id, expires_at);

ALTER TABLE jobs
    ADD COLUMN print_ticket_digest text,
    ADD COLUMN print_ticket_display jsonb
        CHECK (print_ticket_display IS NULL OR jsonb_typeof(print_ticket_display) = 'object');

CREATE INDEX jobs_print_ticket_tenant_idx
    ON jobs (workspace_id, environment_id, print_ticket_digest)
    WHERE print_ticket_digest IS NOT NULL;

ALTER TABLE node_connectors
    ADD COLUMN option_scopes text[] NOT NULL DEFAULT ARRAY[
        'capabilities:read',
        'workflows:read',
        'jobs:submit',
        'job_overrides:safe'
    ]::text[];

ALTER TABLE node_connectors
    ADD CONSTRAINT node_connectors_option_scopes_bounded
    CHECK (cardinality(option_scopes) <= 32);
