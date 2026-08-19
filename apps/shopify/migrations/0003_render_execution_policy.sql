ALTER TABLE shopify_merchant_settings
  ADD COLUMN IF NOT EXISTS render_execution_policy text NOT NULL DEFAULT 'automatic';

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'shopify_merchant_settings_render_execution_policy_check'
      AND conrelid = 'shopify_merchant_settings'::regclass
  ) THEN
    ALTER TABLE shopify_merchant_settings
      ADD CONSTRAINT shopify_merchant_settings_render_execution_policy_check
      CHECK (render_execution_policy IN ('automatic', 'cloud_only', 'prefer_node', 'require_node'));
  END IF;
END
$$;
