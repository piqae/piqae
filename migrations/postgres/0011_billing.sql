CREATE TABLE billing_customers (
    workspace_id text PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    stripe_customer_id text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE billing_subscriptions (
    workspace_id text PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    stripe_subscription_id text UNIQUE,
    plan text NOT NULL DEFAULT 'free' CHECK (plan IN ('free', 'pro')),
    status text NOT NULL DEFAULT 'active',
    current_period_start timestamptz,
    current_period_end timestamptz,
    grace_ends_at timestamptz,
    cancel_at_period_end boolean NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE billing_webhook_receipts (
    event_id text PRIMARY KEY,
    event_type text NOT NULL,
    payload_sha256 text NOT NULL,
    processed_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE workspace_entitlements (
    workspace_id text PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    plan text NOT NULL DEFAULT 'free' CHECK (plan IN ('free', 'pro')),
    included_jobs bigint NOT NULL DEFAULT 100 CHECK (included_jobs >= 0),
    node_limit integer NOT NULL DEFAULT 1 CHECK (node_limit > 0),
    metadata_retention_days integer NOT NULL DEFAULT 7 CHECK (metadata_retention_days > 0),
    document_retention_hours integer NOT NULL DEFAULT 24 CHECK (document_retention_hours >= 0),
    accept_new_cloud_jobs boolean NOT NULL DEFAULT true,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE usage_exports (
    id text PRIMARY KEY,
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    period_start timestamptz NOT NULL,
    period_end timestamptz NOT NULL,
    units bigint NOT NULL CHECK (units >= 0),
    stripe_event_identifier text UNIQUE,
    state text NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'submitted', 'failed')),
    attempts integer NOT NULL DEFAULT 0,
    last_error_code text,
    submitted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, period_start, period_end)
);

CREATE TABLE quota_warning_states (
    workspace_id text NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    period_start timestamptz NOT NULL,
    threshold smallint NOT NULL CHECK (threshold IN (80, 100)),
    notified_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, period_start, threshold)
);
