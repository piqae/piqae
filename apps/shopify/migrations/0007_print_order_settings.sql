ALTER TABLE shopify_merchant_settings
  ADD COLUMN IF NOT EXISTS print_order jsonb NOT NULL DEFAULT
    '{"hierarchy":[],"taxonomyDepth":"family","mixedOrderMode":"dominant"}'::jsonb;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'shopify_merchant_settings_print_order_check'
      AND conrelid = 'shopify_merchant_settings'::regclass
  ) THEN
    ALTER TABLE shopify_merchant_settings
      ADD CONSTRAINT shopify_merchant_settings_print_order_check
      CHECK (
        jsonb_typeof(print_order) = 'object'
        AND octet_length(print_order::text) <= 4096
      );
  END IF;
END $$;
