CREATE TABLE IF NOT EXISTS shopify_installations (
  shop text PRIMARY KEY CHECK (shop ~ '^[a-z0-9][a-z0-9-]*[.]myshopify[.]com$'),
  state text NOT NULL CHECK (state IN ('installed','uninstalled')),
  scopes text NOT NULL DEFAULT '',
  installed_at timestamptz NOT NULL DEFAULT now(),
  uninstalled_at timestamptz,
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS shopify_shop_links (
  shop text PRIMARY KEY REFERENCES shopify_installations(shop) ON DELETE CASCADE,
  piqae_account_id text NOT NULL,
  encrypted_credential text NOT NULL,
  template_revision_id text NOT NULL,
  entitlement_mode text NOT NULL DEFAULT 'existing_piqae' CHECK (entitlement_mode IN ('existing_piqae','shopify_child')),
  plan_handle text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS shopify_webhook_inbox (
  webhook_id text PRIMARY KEY,
  shop text,
  topic text,
  customer_id text,
  resource_id text,
  payload jsonb,
  received_at timestamptz NOT NULL DEFAULT now(),
  processed_at timestamptz,
  attempts integer NOT NULL DEFAULT 0,
  available_at timestamptz NOT NULL DEFAULT now(),
  lease_token uuid,
  lease_expires_at timestamptz,
  last_error text
);
CREATE INDEX IF NOT EXISTS shopify_webhook_inbox_pending_idx ON shopify_webhook_inbox(received_at) WHERE processed_at IS NULL;
CREATE TABLE IF NOT EXISTS shopify_render_ownership (shop text NOT NULL REFERENCES shopify_installations(shop) ON DELETE CASCADE, render_id text NOT NULL, idempotency_key text NOT NULL, order_gid text, customer_gid text, created_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(shop,render_id), UNIQUE(shop,idempotency_key));

CREATE TABLE IF NOT EXISTS shopify_document_templates (
  id uuid PRIMARY KEY, shop text NOT NULL REFERENCES shopify_installations(shop) ON DELETE CASCADE,
  name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200), canonical_spec jsonb NOT NULL,
  liquid_source text CHECK (octet_length(liquid_source) <= 65536), created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(), UNIQUE (shop, name)
);
CREATE TABLE IF NOT EXISTS shopify_document_template_revisions (
  id uuid PRIMARY KEY, template_id uuid NOT NULL REFERENCES shopify_document_templates(id) ON DELETE CASCADE,
  revision integer NOT NULL CHECK (revision > 0), canonical_spec jsonb NOT NULL,
  liquid_source text CHECK (octet_length(liquid_source) <= 65536), created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(template_id, revision)
);
