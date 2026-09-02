CREATE TABLE IF NOT EXISTS shopify_document_usage_events (
  shop text NOT NULL REFERENCES shopify_installations(shop) ON DELETE CASCADE,
  event_key text NOT NULL CHECK(length(event_key) BETWEEN 1 AND 200),
  document_count integer NOT NULL CHECK(document_count BETWEEN 1 AND 10000),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(shop, event_key)
);

CREATE INDEX IF NOT EXISTS shopify_document_usage_events_shop_created_idx
  ON shopify_document_usage_events(shop, created_at DESC);
