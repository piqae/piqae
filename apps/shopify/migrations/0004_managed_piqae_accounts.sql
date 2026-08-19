ALTER TABLE shopify_shop_links
  ALTER COLUMN encrypted_credential DROP NOT NULL,
  ADD COLUMN IF NOT EXISTS piqae_live_environment_id text,
  ADD COLUMN IF NOT EXISTS piqae_test_environment_id text;

-- Pre-release child links held a per-shop bearer credential rather than a
-- platform context. Preserve their access as legacy links; the authenticated
-- Shopify bootstrap replaces them with a managed child account atomically.
UPDATE shopify_shop_links
SET entitlement_mode = 'existing_piqae'
WHERE entitlement_mode = 'shopify_child'
  AND encrypted_credential IS NOT NULL
  AND (piqae_live_environment_id IS NULL OR piqae_test_environment_id IS NULL);

ALTER TABLE shopify_shop_links
  DROP CONSTRAINT IF EXISTS shopify_shop_links_managed_context_check;

ALTER TABLE shopify_shop_links
  ADD CONSTRAINT shopify_shop_links_managed_context_check CHECK (
    (entitlement_mode = 'existing_piqae' AND encrypted_credential IS NOT NULL)
    OR
    (entitlement_mode = 'shopify_child'
      AND encrypted_credential IS NULL
      AND piqae_live_environment_id IS NOT NULL
      AND piqae_test_environment_id IS NOT NULL)
  );

CREATE UNIQUE INDEX IF NOT EXISTS shopify_shop_links_managed_account_idx
  ON shopify_shop_links(piqae_account_id)
  WHERE entitlement_mode = 'shopify_child';
