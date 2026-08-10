process.env.SHOPIFY_API_KEY ??= "fixture-api-key";
process.env.SHOPIFY_API_SECRET ??= "fixture-api-secret-at-least-32-characters";
process.env.SHOPIFY_APP_URL ??= "https://fixture.example.invalid";
process.env.PIQAE_SHOPIFY_CREDENTIAL_KEY ??= Buffer.alloc(32, 1).toString(
  "base64",
);
process.env.PIQAE_SHOPIFY_DOWNLOAD_KEY ??= Buffer.alloc(32, 2).toString(
  "base64",
);
process.env.NODE_ENV = "test";
