# Piqae Shopify app

Embedded Shopify React Router app for direct order-document printing through
Piqae, with an authenticated PDF fallback. It targets Shopify Admin GraphQL and
webhook API `2026-07`; REST Admin API is not used.

The checked-in configuration contains placeholders only. Production startup is
fail-closed until a durable, tenant-fenced `ShopRepository` and Shopify durable
session storage are supplied. Piqae child-account credentials are encrypted at
rest with AES-256-GCM and shop-bound associated data; the wrapping key belongs in
the deployment secret manager.

Printable endpoints authenticate Shopify session tokens and derive the shop
from that context. Callers cannot provide a shop domain. Order IDs are validated
as Shopify Order GIDs and re-fetched with the authenticated GraphQL client.

`read_all_orders` is requested deliberately for historical invoice reprints and
exports beyond Shopify's default order window. It is protected access and must
be approved by Shopify before release. Until approval is present, the product
must explicitly describe and enforce the shorter history available through
`read_orders`; it must not imply that older orders are accessible.

No real Shopify or Piqae credentials are needed for unit tests.

## Production Shopify configuration

`shopify.app.toml` deliberately contains non-deployable placeholder values. Do
not invent or commit a Partner app client ID, extension UID, or production URL.
Before a release, link the repository to the real Partner app and generate the
environment-specific Shopify configuration. Its `application_url` must exactly
match `SHOPIFY_APP_URL`, and its redirect, webhook, app-proxy and customer
extension origins must use that same deployed HTTPS origin. The release owner
must retain the resulting Partner/CLI validation evidence without committing
credentials.

## PostgreSQL migration gate

Run the fresh-database and N−1 upgrade assertions against a disposable PostgreSQL database:

```console
PIQAE_TEST_DATABASE_URL=postgres://... pnpm --filter @piqae/shopify-app test:postgres
```

The test creates two randomly named `piqae_shopify_*` schemas and drops only those exact schemas. It never drops the database. Without `PIQAE_TEST_DATABASE_URL` the local command reports `SKIP` and succeeds. Release and CI jobs must also set `PIQAE_REQUIRE_POSTGRES_TESTS=1`, which turns a missing URL into a failure.
