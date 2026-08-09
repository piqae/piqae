CREATE TABLE IF NOT EXISTS shopify_merchant_settings (
  shop text PRIMARY KEY REFERENCES shopify_installations(shop) ON DELETE CASCADE,
  default_printer_id text, default_template_id text,
  prefer_direct boolean NOT NULL DEFAULT true, offer_pdf boolean NOT NULL DEFAULT true,
  metafield_allowlist text[] NOT NULL DEFAULT '{}', retention_days integer NOT NULL DEFAULT 30 CHECK(retention_days BETWEEN 1 AND 365),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS shopify_workflow_templates (
  id uuid NOT NULL, shop text NOT NULL REFERENCES shopify_installations(shop) ON DELETE CASCADE,
  name text NOT NULL CHECK(length(name) BETWEEN 1 AND 200), kind text NOT NULL CHECK(kind IN ('invoice','packing_slip','receipt','returns','credit_note','custom')),
  page_size text NOT NULL CHECK(page_size IN ('A4','A5','Letter','80mm')), state text NOT NULL CHECK(state IN ('draft','published')),
  source text NOT NULL CHECK(octet_length(source)<=65536), revision integer NOT NULL DEFAULT 1 CHECK(revision>0),
  updated_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(id,shop)
);
CREATE INDEX IF NOT EXISTS shopify_workflow_templates_shop_updated_idx ON shopify_workflow_templates(shop,updated_at DESC);
CREATE TABLE IF NOT EXISTS shopify_workflow_template_revisions (
  template_id uuid NOT NULL, shop text NOT NULL, revision integer NOT NULL CHECK(revision>0),
  name text NOT NULL, kind text NOT NULL, page_size text NOT NULL,
  source text NOT NULL CHECK(octet_length(source)<=65536), created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(template_id,shop,revision),
  FOREIGN KEY(template_id,shop) REFERENCES shopify_workflow_templates(id,shop) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS shopify_automation_rules (
  id uuid NOT NULL, shop text NOT NULL REFERENCES shopify_installations(shop) ON DELETE CASCADE,
  name text NOT NULL CHECK(length(name) BETWEEN 1 AND 200), trigger_event text NOT NULL CHECK(trigger_event IN ('order_paid','order_created','fulfillment_created','refund_created')),
  delivery text NOT NULL CHECK(delivery IN ('printer','email')), template_id uuid NOT NULL, destination text NOT NULL CHECK(length(destination)<=320),
  enabled boolean NOT NULL DEFAULT false, updated_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(id,shop),
  FOREIGN KEY(template_id,shop) REFERENCES shopify_workflow_templates(id,shop) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS shopify_print_activity (
  id uuid NOT NULL, shop text NOT NULL REFERENCES shopify_installations(shop) ON DELETE CASCADE,
  order_name text NOT NULL, document_name text NOT NULL, destination text NOT NULL,
  state text NOT NULL CHECK(state IN ('accepted','printing','reported_complete','uncertain','failed')),
  created_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(id,shop)
);
CREATE INDEX IF NOT EXISTS shopify_print_activity_shop_created_idx ON shopify_print_activity(shop,created_at DESC);
CREATE TABLE IF NOT EXISTS shopify_billing_state (
  shop text PRIMARY KEY REFERENCES shopify_installations(shop) ON DELETE CASCADE,
  mode text NOT NULL CHECK(mode IN ('existing_piqae','shopify_child')), plan_handle text NOT NULL CHECK(plan_handle IN ('free','starter','growth','scale')),
  used_count integer NOT NULL DEFAULT 0 CHECK(used_count>=0), plan_limit integer NOT NULL CHECK(plan_limit>0),
  status text NOT NULL CHECK(status IN ('active','approval_required')), updated_at timestamptz NOT NULL DEFAULT now()
);
